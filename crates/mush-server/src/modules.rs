//! M3 module wiring: which [`GameModule`]s the server boots with, their
//! persisted-data load path, and the per-namespace module migrations.
//!
//! Boot order (driven from `main.rs` / test rigs via [`boot_modules`]):
//!
//! 1. [`Engine::compose_modules`] has already run every module's `register`
//!    (commands, hooks, ticks, state slots) against the one registry and
//!    frozen mechanics before world hydration; [`Engine::from_composition`]
//!    then installed the result on the loaded world.
//! 2. [`run_module_migrations`] brings each registered namespace's
//!    persisted documents up to its module's current schema (BGSAVE first —
//!    the mandatory pre-migrate snapshot), **before** any module parses
//!    them.
//! 3. The boot loader partitions the coordinator-reserved `action-state/*`
//!    documents away from ordinary module data and atomically hydrates them
//!    through their registered typed decoders.
//! 4. The loader supplies every remaining kv doc (minus the
//!    reserved [`SCHEMA_KEY`]) plus the streams the module names via
//!    [`GameModule::boot_stream_requests`].
//! 5. [`Engine::modules_loaded`] hands each module its
//!    [`ModuleBootData`]; whatever the modules persist during boot
//!    (first-boot seeding, orphan-doc cleanup) is drained and written
//!    through **durably, acked** before the server starts serving.
//!
//! The module list itself is [`server_modules`]: comsys first (its
//! [`AliasStage`](mush_core::AliasStage) is the earliest alias stage in
//! dispatch), then canon, org, faction, scenes, simulation, tactics, economy,
//! mail, jobs, weather, then script (both M5).
//! Their persistence namespaces are disjoint, so the order is otherwise
//! cosmetic — but it is the registration order hooks and observers fire in,
//! and it is the order `on_world_loaded` runs in, so keep it stable.
//!
//! Two orderings are **load-bearing** and documented at their entries: org
//! before economy (economy authorizes through org), and — since E3c — sim
//! before economy, because the economy's boot asks `mush-sim`'s custody
//! provider about interior vault backings and a module state slot exists,
//! empty, from registration onward.
//!
//! Script goes **last** deliberately. Its object `$`-command matcher is a
//! post-command-table [`CommandFallback`](mush_core::CommandFallback), not an
//! alias stage, so it never competes with comsys for dispatch order. Running
//! its `register` after the others means its `on_world_loaded` snapshots a
//! registry that already carries every other module's attr namespaces (the
//! off-loop session enforces module-namespace ownership against that
//! snapshot), and its `AttrSet`/`Moved`/`Said` observers fire after the
//! game-system modules' observers for the same event — the intended order,
//! since a scripted reflex should see the world the built-ins already left.
//!
//! The M0 `--repl` stays module-free on purpose (offline poking of the core
//! surface; it has no store to load module data from).

use std::collections::HashMap;
use std::sync::Arc;

use mush_core::{
    ActionStateBootRecord, Effects, Engine, GameModule, Migration, ModuleBootData, Outbox,
    is_action_state_persistence_key,
};
use mush_llm::LlmService;
use mush_ruleset::Ruleset;
use mush_store::{ModuleOp, ModuleWrite, ObjectStore, WriteBatch};
use mush_worldgen::WorldgenModule;
use smol_str::SmolStr;

/// Reserved kv key per module namespace carrying `{"version": N}` — the
/// namespace's schema version for [`run_module_migrations`]. Filtered out
/// of [`ModuleBootData::kv`] so modules never see it.
pub const SCHEMA_KEY: &str = "schema";

/// The built-in game modules a production `mushd` boots with, in
/// registration order, with **default** module config. Config-selected
/// [`ruleset_modules`] are appended **after** these (see `main.rs`); hand the
/// concatenation to [`Engine::with_modules`].
#[must_use]
pub fn server_modules() -> Vec<Box<dyn GameModule>> {
    server_modules_with(
        mush_weather::WeatherConfig::default(),
        mush_economy::EconomyConfig::default(),
        &ruleset_wod::OnboardingConfig::default(),
    )
}

/// [`server_modules`] with explicit module configs: the `[weather]` knobs (the M10
/// flavor-variant sizing), the `[economy]` knobs (E4's mint bound on a ruleset's
/// faucet-funded obligations), and `[onboarding]`'s start rooms (E5b's reserved-room
/// list). `main.rs` uses this so those sections in `mushd.toml` actually reach their
/// modules; every other caller (tests) takes the crate defaults via
/// [`server_modules`].
#[must_use]
pub fn server_modules_with(
    weather: mush_weather::WeatherConfig,
    economy: mush_economy::EconomyConfig,
    onboarding: &ruleset_wod::OnboardingConfig,
) -> Vec<Box<dyn GameModule>> {
    vec![
        Box::new(mush_comsys::ComsysModule::new()),
        Box::new(mush_canon::CanonModule::new()),
        // E1: organization identity + scoped authorization (the economy's
        // ownership/permission substrate). E1c wires the faction-standing predicate
        // provider so an org AccessPolicy can gate on live faction membership /
        // officer / intel standing (the faction level model stays faction-side).
        // The provider is evaluated at authorize time, never at boot, so org may
        // load BEFORE faction — and since the post-E3 owner decision it does: the
        // faction module validates every `Faction.org` backing link against loaded
        // org state at its own boot (reporting a missing org or a two-factions-one-
        // org conflict loudly, never unbinding). Org's boot reads no faction state.
        // E3b2 freezes two more org tables at wiring: the capability VOCABULARY (so
        // `@org/grant` refuses a typo instead of storing a grant that authorizes
        // nothing), and the scope-use binder table (so `@org/scope retire` can refuse
        // a compartment the economy still binds — org sits below economy and cannot
        // look for an account itself).
        Box::new(
            mush_org::OrgModule::with_predicates(vec![std::sync::Arc::new(
                mush_faction::FactionStandingProvider,
            )])
            .with_capabilities(org_capability_vocabulary())
            .with_scope_use(vec![std::sync::Arc::new(mush_economy::EconomyScopeUse)]),
        ),
        // After org (see above): `Faction.org` links are checked at faction boot.
        Box::new(mush_faction::FactionModule::new()),
        Box::new(mush_scenes::ScenesModule::new()),
        Box::new(mush_sim::SimulationModule::new()),
        Box::new(mush_tactics::TacticsModule::new()),
        // E2: the accountable-value engine — the double-entry journal, accounts, and
        // the strict durable money ack-gate. After org (it authorizes through it).
        // E3b1 freezes the custody provider table at wiring: the built-in container
        // provider is appended by the module itself, and E3c adds the interior
        // provider for `@vault/create … interior:<space>`.
        //
        // E3c moved economy **after** `SimulationModule` (org must still come
        // first; scenes and sim read no economy state at boot). Boot order is
        // `on_world_loaded` order, and `SpatialState` exists — EMPTY — from
        // registration onward, so an economy booting first would ask the interior
        // provider about a vault backing and get a definite `Some(false)` from a
        // module that simply has not loaded its documents yet: every interior
        // vault would boot `Frozen` and every deposited asset would latch
        // `custody-mismatch`. With sim ahead of it, the economy's custody
        // reconciliation and vault-backing validation see real state, and a
        // `Some(false)` can be trusted as the drift it claims to be.
        Box::new(
            mush_economy::EconomyModule::with_custody(vec![Arc::new(
                mush_sim::InteriorCustodyProvider,
            )])
            .with_config(economy)
            // E4c (08c §6.1): three more wiring-frozen tables, none of which
            // changes the module ORDER above. The fact table answers a contract's
            // `Fact` conditions (jobs today; doc 10 §15's sim/tactics boundary
            // facts next), the observer table carries the canon evidence bridge,
            // and the title-issuer table is empty until a ruleset registers one —
            // `@asset/title` stays the staff path meanwhile.
            .with_facts(vec![Arc::new(mush_jobs::JobsFactProvider)])
            // E5b: housing's exact-mode mirror joins the canon bridge. It hears the
            // obligation and contract transitions a lease is made of; abstract mode
            // does not need it at all.
            .with_observers(vec![
                Arc::new(crate::econ_canon::CanonEvidenceBridge),
                Arc::new(mush_housing::HousingEconomyObserver),
            ])
            // E5b: `system:wod.housing` mints the title record on a property's anchor
            // room through `mush_economy::issue_title` (D-E3-13, 08d §2). The same
            // principal is wired into `HousingModule::with_issuer` below, and
            // `issue_title` refuses any issuer that is not on BOTH lists.
            //
            // 08d §2 says the server registers it "only when the WoD ruleset is
            // configured"; E5c owns that knob (`[wod.economy].housing`), and until it
            // lands the issuer is wired unconditionally. It is inert without housing:
            // a registered issuer that nothing calls mints nothing.
            .with_title_issuers(vec![housing_title_issuer()])
            // E4c verify fold (E1): the canon kinds that count as a
            // **producer-declared outcome**. Deliberately empty — `scene.concluded`
            // is a lifecycle record any participant can produce, so a payee gated on
            // one could seal their own settlement. E6b registers `scene.outcome`
            // here once `mush-scenes` can seal one under facilitator authority;
            // until then every `canon:` attestor set is refused at `@contract/offer`.
            .with_canon_outcome_kinds(Vec::new()),
        ),
        Box::new(mush_mail::MailModule),
        Box::new(mush_jobs::JobsModule::new()),
        // E5: housing boots after `{org, sim, tactics, economy, jobs}` (08a decision
        // 19, 08d §2). `on_world_loaded` runs in list order and housing's boot
        // reconciliation asks all four of them questions: org for the landlord
        // compartment, the economy for cadence positions and rent obligations, jobs
        // for the story queue, and the world itself for the rooms.
        // E5b verify fold (H4): the reserved-room list is wired **now**, not left to
        // E5c. D-E5-12 says a chargen start room is never somebody's living room, and
        // housing enforces it without a `mush-config` edge by being told — so until
        // the host tells it, `@property/create <name>=<the start room>` was accepted
        // and every fresh character would have walked out of chargen into a private
        // residence the access hook then answered for. `[onboarding]` owns the two
        // rooms; Limbo is on the list belt-and-braces (housing refuses it anyway).
        Box::new(
            mush_housing::HousingModule::new()
                .with_issuer(housing_title_issuer())
                .with_reserved_rooms([
                    onboarding.start_room,
                    onboarding.start_room_unapproved,
                    mush_core::LIMBO,
                ]),
        ),
        Box::new(mush_weather::WeatherModule::with_config(weather)),
        Box::new(mush_script::ScriptModule::new()),
    ]
}

/// The system principal housing mints property titles under (08d §2, D-E3-13).
///
/// It is named in exactly two places — `EconomyModule::with_title_issuers` and
/// `HousingModule::with_issuer` — and `mush_economy::issue_title` refuses an issuer
/// that is not on the economy's list, so a typo in either one is a loud refusal at
/// `@property/create` rather than a silent capability.
#[must_use]
pub fn housing_title_issuer() -> mush_org::SystemPrincipalId {
    mush_org::SystemPrincipalId::new("wod.housing")
        .expect("a built-in system principal id is well-formed")
}

/// The capability vocabulary this server recognizes (E3b2 §4.8.4 / §4.10): every
/// capability the economy asks org about (`mush_economy::CAPABILITIES`) plus the two
/// org-administrative ones the org module's own doctrine names.
///
/// It is a **typo catcher**, not an authorization gate — `authorize` matches
/// capability ids by equality, so an unknown one already grants nothing; what this
/// buys is that granting `economy.acount.post` fails loudly at the command instead
/// of quietly producing an inert record. Add to it whenever a module starts asking
/// about a new capability.
#[must_use]
pub fn org_capability_vocabulary() -> std::collections::BTreeSet<mush_org::CapabilityId> {
    mush_economy::CAPABILITIES
        .iter()
        .copied()
        .chain(mush_housing::CAPABILITIES.iter().copied())
        .chain(["org.role.manage", "org.grant.delegate"])
        .map(|c| mush_org::CapabilityId::new(c).expect("a built-in capability id is well-formed"))
        .collect()
}

/// The ruleset module(s) named in `[rulesets].enabled`, in listed order.
///
/// Every ruleset crate is compiled into `mushd`; this maps the configured
/// ids to their modules. Registered **after** [`server_modules`] so a
/// ruleset's commands, panel feeds, and job queues wire cleanly and its
/// approval payload handlers (registered in `on_world_loaded`) run after the
/// jobs module's state slot exists.
///
/// A ruleset's sheets ride the object attr write-through (treaty ruling 7),
/// so they load with the world — a ruleset registers no boot namespace and
/// needs no separate boot-data load here.
///
/// An unknown id is a hard error (consistent with the rest of config: a
/// silently-ignored typo is worse than not starting). The ids available as of
/// M10b are `"wod"` (oWoD20) and `"shadowrun"` (SR5); duplicates are rejected.
///
/// **Single-ruleset-per-server.** Every ruleset registers the same core
/// command surface (`+roll`/`+sheet`/`+chargen`/…), so two of them cannot share
/// one registry — enabling more than one is a configuration error caught here
/// with a clear message (rather than surfacing downstream as an opaque
/// command-collision at registration). Enable exactly one ruleset, or none (a
/// bare comsys/mail server). Per-character ruleset dispatch across systems is a
/// future concern.
pub fn ruleset_modules(
    enabled: &[String],
    onboarding: &ruleset_wod::OnboardingConfig,
) -> Result<Vec<Box<dyn GameModule>>, String> {
    let mut seen: Vec<String> = Vec::new();
    let mut mods: Vec<Box<dyn GameModule>> = Vec::new();
    for id in enabled {
        let key = id.trim().to_lowercase();
        if seen.contains(&key) {
            return Err(format!("[rulesets].enabled lists {id:?} more than once"));
        }
        match key.as_str() {
            "wod" => {
                mods.push(Box::new(
                    ruleset_wod::WodRuleset::new().onboarding(onboarding.clone()),
                ));
            }
            "shadowrun" => {
                // SR5 (M10b). Single-ruleset-per-server: wod and shadowrun both
                // register +roll/+sheet/+chargen, so a server runs one of them.
                // SR5 has no onboarding gate wired yet.
                mods.push(Box::new(ruleset_shadowrun::ShadowrunRuleset::new()));
            }
            _ => {
                return Err(format!(
                    "[rulesets].enabled: unknown ruleset {id:?} (available: \"wod\", \"shadowrun\")"
                ));
            }
        }
        seen.push(key);
    }
    // Single ruleset per server: two rulesets would collide on the shared
    // +roll/+sheet/+chargen command names. Fail here with an explanation
    // instead of letting Engine::with_modules report an opaque collision.
    if mods.len() > 1 {
        return Err(format!(
            "[rulesets].enabled selects {} rulesets ({enabled:?}), but rustmush runs one \
             ruleset per server — they all register +roll/+sheet/+chargen, so more than one \
             collides. Enable exactly one (e.g. [\"shadowrun\"]) or none.",
            mods.len()
        ));
    }
    Ok(mods)
}

/// The M7 worldgen module, backed by the shared [`LlmService`] handle.
///
/// `main.rs` inserts it into the boot list **after** [`server_modules`] (so it
/// registers after the built-ins) and before the config-selected
/// [`ruleset_modules`]. It registers regardless of whether the LLM is enabled:
/// with `[llm].enabled = false` the service's kill switch is engaged, so
/// `@gen` still parses and defers, and the off-loop generator re-enters a
/// "disabled" outcome (a keyless clone boots and its worldgen commands answer
/// cleanly rather than 404). Its persistence namespace (`worldgen`) is disjoint
/// from every other module's, and it loads its drafts through the same
/// boot-data path as the rest ([`boot_modules`]).
#[must_use]
pub fn worldgen_module(llm: Arc<LlmService>) -> Box<dyn GameModule> {
    Box::new(WorldgenModule::new(llm))
}

/// [`worldgen_module`] with an explicit worldgen config (`[worldgen]` knobs —
/// the M10 `@gen/building` room cap). `main.rs` uses this so `[worldgen]` in
/// `mushd.toml` reaches the module; tests take the crate defaults via
/// [`worldgen_module`].
#[must_use]
pub fn worldgen_module_with(
    llm: Arc<LlmService>,
    config: mush_worldgen::WorldgenConfig,
) -> Box<dyn GameModule> {
    Box::new(WorldgenModule::with_config(llm, config))
}

/// The M8 NPC agent module. Registered after worldgen and before the rulesets.
/// `grounding` supplies the ruleset sheet summary + ability tools through the
/// generic [`RulesetAgent`] (whichever ruleset is enabled); `None` runs NPCs
/// dialogue-only.
#[must_use]
pub fn npc_module(grounding: Option<Arc<dyn mush_npc::AgentGrounding>>) -> Box<dyn GameModule> {
    match grounding {
        Some(g) => Box::new(mush_npc::NpcModule::with_grounding(g)),
        None => Box::new(mush_npc::NpcModule::new()),
    }
}

/// Build the NPC↔ruleset grounding adapter for the enabled ruleset, if one
/// grounds NPC agents. `main.rs` passes it to [`npc_module`]. Grounding
/// dispatches through the generic [`RulesetAgent`], so **every** ruleset — not
/// just WoD — grounds its NPCs: a shadowrun-only server grounds SR5 NPCs
/// (sheet summary + `sr_roll` + `@agent/sheet`) exactly as a wod server grounds
/// WoD NPCs. A ruleset-less config leaves NPCs dialogue-only (`None`).
///
/// Single-ruleset-per-server (see [`ruleset_modules`]), so at most one entry
/// matches; the first recognized id wins.
#[must_use]
pub fn npc_agent_adapter(enabled: &[String]) -> Option<Arc<dyn mush_npc::AgentGrounding>> {
    for id in enabled {
        match id.trim().to_ascii_lowercase().as_str() {
            "wod" => {
                return Some(Arc::new(RulesetAgent::new(ruleset_wod::WodRuleset::new())));
            }
            "shadowrun" => {
                return Some(Arc::new(RulesetAgent::new(
                    ruleset_shadowrun::ShadowrunRuleset::new(),
                )));
            }
            _ => {}
        }
    }
    None
}

/// A generic NPC↔ruleset grounding adapter (treaty ruling 6) over any
/// [`Ruleset`], running **on the world loop**. This is the seam that carries the
/// milestone's "the plugin API generalises past WoD" claim end-to-end: the NPC
/// host dispatches grounding through the [`Ruleset`] trait, so any enabled
/// ruleset grounds its NPCs with no NPC-wiring change — the earlier
/// `WodAgent` hardcoded the concrete `WodRuleset`, which left a shadowrun-only
/// server's NPCs dialogue-only despite SR5 fully implementing the seam.
///
/// - [`profile`](mush_npc::AgentGrounding::profile) reads the NPC's `sheet:<id>`
///   document (via a [`SheetStore`](mush_ruleset::SheetStore) over the cloned
///   schema) and grounds it via [`Ruleset::agent_profile`].
/// - [`invoke_ability`](mush_npc::AgentGrounding::invoke_ability) executes a
///   validated ability (`wod_roll`, `sr_roll`, …) via
///   [`Ruleset::invoke_ability`], which reads the NPC's own sheet, rolls on the
///   shared server RNG, and emits to the room.
/// - [`init_sheet`](mush_npc::AgentGrounding::init_sheet) backs `@agent/sheet`
///   via [`Ruleset::init_sheet`].
///
/// The held ruleset is grounding-only — it reads each NPC's own sheet from the
/// world and rolls on the shared world state, carrying no per-character state.
pub struct RulesetAgent<R: Ruleset> {
    ruleset: R,
    schema: Arc<mush_ruleset::schema::SheetSchema>,
}

impl<R: Ruleset> RulesetAgent<R> {
    /// A grounding adapter over `ruleset`, capturing its sheet schema for reads.
    #[must_use]
    pub fn new(ruleset: R) -> RulesetAgent<R> {
        let schema = Arc::new(ruleset.sheet_schema().clone());
        RulesetAgent { ruleset, schema }
    }
}

impl<R: Ruleset + Send + Sync + 'static> mush_npc::AgentGrounding for RulesetAgent<R> {
    fn profile(
        &self,
        w: &mush_core::World,
        npc: mush_core::ObjId,
    ) -> Option<mush_ruleset::AgentCapabilityProfile> {
        let doc = mush_ruleset::SheetStore::new(Arc::clone(&self.schema)).get(w, npc)?;
        Some(self.ruleset.agent_profile(&doc))
    }

    fn invoke_ability(
        &self,
        w: &mut mush_core::World,
        out: &mut mush_core::Outbox,
        npc: mush_core::ObjId,
        name: &str,
        input: serde_json::Value,
    ) -> Result<mush_ruleset::AbilityOutcome, mush_ruleset::AbilityError> {
        self.ruleset.invoke_ability(w, out, npc, name, input)
    }

    fn init_sheet(
        &self,
        w: &mut mush_core::World,
        npc: mush_core::ObjId,
        template: &str,
        by: mush_core::ObjId,
    ) -> Result<String, String> {
        self.ruleset.init_sheet(w, npc, template, by)
    }
}

/// Read and partition one namespace without invoking module-supplied boot
/// callbacks. Typed state must be validated globally before callbacks run.
async fn load_one_namespace(
    store: &dyn ObjectStore,
    ns: &str,
) -> Result<(ModuleBootData, Vec<ActionStateBootRecord>), String> {
    let documents = store
        .module_json_list(ns)
        .await
        .map_err(|e| format!("listing module namespace {ns:?}: {e}"))?;
    let mut kv = Vec::new();
    let mut action_state = Vec::new();
    for (key, value) in documents {
        if key == SCHEMA_KEY {
            continue;
        }
        if is_action_state_persistence_key(&key) {
            action_state.push(ActionStateBootRecord::new(ns, key, value));
        } else {
            kv.push((SmolStr::new(key), value));
        }
    }
    Ok((
        ModuleBootData {
            kv,
            streams: HashMap::new(),
        },
        action_state,
    ))
}

/// Only after typed state has hydrated, let modules select their bounded
/// stream tails from already-partitioned ordinary JSON data.
async fn load_requested_streams(
    store: &dyn ObjectStore,
    modules: &[Box<dyn GameModule>],
    boot: &mut HashMap<SmolStr, ModuleBootData>,
) -> Result<(), String> {
    for module in modules {
        let ns = module.name();
        let data = boot
            .get_mut(ns)
            .expect("every composed module namespace was loaded in the first pass");
        let mut streams = HashMap::new();
        let stream_requests = module
            .boot_stream_requests(&data.kv)
            .map_err(|e| format!("validating module stream requests for {ns:?}: {e}"))?;
        for (key, count) in stream_requests {
            let entries = store
                .stream_range(ns, &key, count)
                .await
                .map_err(|e| format!("loading module stream {ns}:{key}: {e}"))?;
            streams.insert(key, entries);
        }
        data.streams = streams;
    }
    Ok(())
}

/// Load only ordinary module boot data. The full public boot path is
/// [`boot_modules`], which also hydrates the reserved typed-state partition.
#[cfg(test)]
async fn load_module_boot(
    store: &dyn ObjectStore,
    modules: &[Box<dyn GameModule>],
) -> Result<HashMap<SmolStr, ModuleBootData>, String> {
    let (mut boot, _) = load_module_boot_with_action_state(store, modules).await?;
    load_requested_streams(store, modules, &mut boot).await?;
    Ok(boot)
}

async fn load_module_boot_with_action_state(
    store: &dyn ObjectStore,
    modules: &[Box<dyn GameModule>],
) -> Result<(HashMap<SmolStr, ModuleBootData>, Vec<ActionStateBootRecord>), String> {
    let mut boot = HashMap::new();
    let mut action_state = Vec::new();
    for module in modules {
        let (module_boot, mut module_action_state) =
            load_one_namespace(store, module.name()).await?;
        boot.insert(SmolStr::new(module.name()), module_boot);
        action_state.append(&mut module_action_state);
    }
    Ok((boot, action_state))
}

/// The schema version a namespace's [`SCHEMA_KEY`] doc claims, if any.
async fn ns_version(store: &dyn ObjectStore, ns: &str) -> Result<Option<u32>, String> {
    let doc = store
        .module_json_get(ns, SCHEMA_KEY)
        .await
        .map_err(|e| format!("reading {ns} schema doc: {e}"))?;
    Ok(doc
        .and_then(|d| d.get("version").and_then(serde_json::Value::as_u64))
        .map(|v| u32::try_from(v).unwrap_or(u32::MAX)))
}

/// Write one batch of module ops through the store, ack awaited (boot runs
/// before the world task, so it writes directly).
async fn write_ops(store: &dyn ObjectStore, ops: Vec<ModuleOp>) -> Result<(), String> {
    const BOOT_SERIAL: u64 = 0;
    let batch = WriteBatch {
        serial: BOOT_SERIAL,
        durable: true,
        module_ops: ops,
        ..WriteBatch::default()
    };
    let receipt = store
        .write(batch)
        .await
        .map_err(|e| format!("module boot write: {e}"))?;
    if receipt.serial != BOOT_SERIAL {
        return Err(format!(
            "module boot write: expected receipt serial {BOOT_SERIAL}, got {}",
            receipt.serial
        ));
    }
    Ok(())
}

/// A `JsonSet` stamping a namespace's schema version.
fn stamp(ns: &str, version: u32) -> ModuleOp {
    ModuleOp {
        ns: SmolStr::new(ns),
        op: ModuleWrite::JsonSet {
            key: SmolStr::new(SCHEMA_KEY),
            value: serde_json::json!({ "version": version }),
        },
    }
}

/// Run every module's registered per-namespace migrations (collected via
/// `Registrar::migrations`, exposed by
/// [`Registry::migrations`](mush_core::Registry::migrations)) against the
/// store, following the same rules as the core schema
/// ([`mush_store::run_migrations`]):
///
/// - a **fresh namespace** (no schema doc, no documents) is stamped to the
///   latest registered version without running anything;
/// - an unstamped namespace **with** documents is pre-versioning data at
///   v0: every migration runs;
/// - a namespace **newer than the build** refuses to boot;
/// - before any migration actually runs, `bgsave` is triggered once (the
///   mandatory pre-migrate snapshot);
/// - each step transforms every document in the namespace (minus the
///   schema doc) and commits them **with** the version bump in one batch,
///   so a crash between steps resumes exactly where it stopped.
pub async fn run_module_migrations(
    store: &dyn ObjectStore,
    migrations: &HashMap<SmolStr, Vec<Migration>>,
) -> Result<(), String> {
    let mut namespaces: Vec<&SmolStr> = migrations.keys().collect();
    namespaces.sort(); // deterministic order across boots
    let mut snapshotted = false;
    for ns in namespaces {
        let migs = &migrations[ns];
        let Some(latest) = migs.last().map(|m| m.version) else {
            continue;
        };
        let mut prev = 0u32;
        for m in migs {
            if m.version <= prev {
                return Err(format!(
                    "module {ns}: migration set not strictly ascending at v{}",
                    m.version
                ));
            }
            prev = m.version;
        }
        let current = match ns_version(store, ns).await? {
            Some(v) => v,
            None => {
                let docs = store
                    .module_json_list(ns)
                    .await
                    .map_err(|e| format!("listing module namespace {ns:?}: {e}"))?;
                if docs
                    .iter()
                    .all(|(key, _)| key == SCHEMA_KEY || is_action_state_persistence_key(key))
                {
                    // Fresh namespace: stamp to latest, run nothing.
                    write_ops(store, vec![stamp(ns, latest)]).await?;
                    tracing::info!(ns = %ns, version = latest, "fresh module namespace stamped");
                    continue;
                }
                0 // pre-versioning data
            }
        };
        if current > latest {
            return Err(format!(
                "module {ns}: stored schema v{current} is newer than this build's \
                 v{latest}; refusing to run (restore from backup to downgrade)"
            ));
        }
        if current == latest {
            continue;
        }
        if !snapshotted {
            store
                .bgsave()
                .await
                .map_err(|e| format!("pre-migrate BGSAVE failed: {e}"))?;
            snapshotted = true;
        }
        for m in migs.iter().filter(|m| m.version > current) {
            tracing::info!(ns = %ns, to = m.version, desc = %m.description,
                "running module migration");
            let docs = store
                .module_json_list(ns)
                .await
                .map_err(|e| format!("listing module namespace {ns:?}: {e}"))?;
            let mut ops = Vec::new();
            for (key, value) in docs {
                if key == SCHEMA_KEY || is_action_state_persistence_key(&key) {
                    continue;
                }
                let migrated = (m.apply)(&key, value).map_err(|e| {
                    format!("module {ns} migration to v{}: doc {key:?}: {e}", m.version)
                })?;
                ops.push(ModuleOp {
                    ns: ns.clone(),
                    op: ModuleWrite::JsonSet {
                        key: SmolStr::new(key),
                        value: migrated,
                    },
                });
            }
            // The version bump commits with the transformed docs: a crash
            // between steps re-runs from exactly the stored version.
            ops.push(stamp(ns, m.version));
            write_ops(store, ops).await?;
        }
    }
    Ok(())
}

/// The full module boot sequence over an already-constructed engine (see
/// the module docs): migrations → boot data → `modules_loaded` → durable
/// write-through of whatever boot persisted. Call between
/// [`Engine::from_composition`] (or the compatibility
/// [`Engine::with_modules`] wrapper) and spawning the world task.
pub async fn boot_modules(engine: &mut Engine, store: &dyn ObjectStore) -> Result<(), String> {
    run_module_migrations(store, engine.registry.migrations()).await?;
    let (mut boot, action_state) =
        load_module_boot_with_action_state(store, engine.modules()).await?;
    engine
        .hydrate_action_state(action_state)
        .map_err(|error| format!("action-state boot failed: {error}"))?;
    load_requested_streams(store, engine.modules(), &mut boot).await?;
    let mut out = Outbox::new();
    let mut fx = Effects::new();
    engine
        .modules_loaded(boot, &mut out, &mut fx)
        .map_err(|e| format!("module boot failed: {e}"))?;
    // Nothing is connected yet; boot-time output has nowhere to go.
    let events = out.take();
    if !events.is_empty() {
        tracing::debug!(
            count = events.len(),
            "module boot output dropped (no sessions yet)"
        );
    }
    debug_assert!(fx.is_empty(), "modules must not defer effects at boot");
    // First-boot seeding / orphan cleanup must be durable before serving.
    let ops = engine.take_module_ops();
    if let Some((namespace, key)) = ops.iter().find_map(|op| {
        let key = match &op.op {
            ModuleWrite::JsonSet { key, .. }
            | ModuleWrite::Del { key }
            | ModuleWrite::StreamAppend { key, .. } => key,
        };
        is_action_state_persistence_key(key).then_some((op.ns.as_str(), key.as_str()))
    }) {
        return Err(format!(
            "module boot attempted coordinator-reserved persistence key {namespace}:{key}"
        ));
    }
    if !ops.is_empty() {
        write_ops(store, ops).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use mush_core::{
        AuthoritativeBundle, AuthoritativeBundleDraft, AuthoritativeKey, CanonicalHash,
        CanonicalId, CommitToken, DefinitionDescriptor, DefinitionId, DefinitionKind,
        DefinitionRegistryBuilder, GOD, IdempotencyKey, IdempotencyScope, ModError, OwnerId,
        PreparedCausalRef, PrincipalId, Registrar, ReplayWrite, RequestUid, RulesValue,
        ScopedIdempotencyKey, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId, World,
    };
    use mush_store::{AuthoritativeStore, InMemoryStore};

    /// A module that persists one doc under ns "toy" and registers a v1→v2
    /// migration renaming field `a` to `b`.
    struct ToyMod {
        loaded: Vec<(SmolStr, serde_json::Value)>,
    }

    impl GameModule for ToyMod {
        fn name(&self) -> &'static str {
            "toy"
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            reg.migrations(
                "toy",
                vec![
                    Migration {
                        version: 1,
                        description: "initial".into(),
                        apply: Arc::new(|_key, v| Ok(v)),
                    },
                    Migration {
                        version: 2,
                        description: "rename a to b on doc:* only".into(),
                        // The key parameter routes heterogeneous
                        // namespaces: only doc:* documents are reshaped.
                        apply: Arc::new(|key: &str, mut v: serde_json::Value| {
                            if key.starts_with("doc:")
                                && let Some(obj) = v.as_object_mut()
                                && let Some(a) = obj.remove("a")
                            {
                                obj.insert("b".into(), a);
                            }
                            Ok(v)
                        }),
                    },
                ],
            )?;
            Ok(())
        }

        fn on_world_loaded(
            &mut self,
            _ctx: &mut mush_core::ModuleCtx<'_>,
            data: ModuleBootData,
        ) -> Result<(), ModError> {
            self.loaded = data.kv;
            Ok(())
        }
    }

    fn toy_engine() -> Engine {
        Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![Box::new(ToyMod { loaded: Vec::new() })],
        )
        .unwrap()
    }

    #[derive(Clone, Debug, serde::Deserialize, Eq, PartialEq)]
    #[serde(deny_unknown_fields)]
    struct BootState {
        amount: u8,
    }

    struct StateBootMod {
        handle: Arc<Mutex<Option<StateFamilyHandle<u64, BootState>>>>,
        streams_requested: Arc<AtomicBool>,
        loaded: Arc<AtomicBool>,
    }

    impl GameModule for StateBootMod {
        fn name(&self) -> &'static str {
            "state_boot_test"
        }

        fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
            let descriptor = StateFamilyDescriptor::new(
                StateFamilyId::new("boot").expect("static family id"),
                CanonicalId::new("u64.decimal.v1").expect("static codec id"),
                CanonicalId::new("boot_state.v1").expect("static schema id"),
                1,
                CanonicalId::new("boot_state.decode_json.v1").expect("static decoder id"),
                CanonicalId::new("boot_state.range.v1").expect("static validator id"),
                CanonicalId::new("boot_state.json.v1").expect("static serializer id"),
            )
            .map_err(|error| ModError::msg(error.to_string()))?;
            let handle = reg.state_family(
                descriptor,
                |key: &u64| Ok(key.to_string()),
                |value: &serde_json::Value| {
                    serde_json::from_value(value.clone()).map_err(|error| error.to_string())
                },
                |_value: &BootState| Ok(()),
                |value: &BootState| Ok(serde_json::json!({ "amount": value.amount })),
            )?;
            *self.handle.lock().expect("handle mutex poisoned") = Some(handle);
            Ok(())
        }

        fn boot_stream_requests(
            &self,
            _kv: &[(SmolStr, serde_json::Value)],
        ) -> Result<Vec<(SmolStr, u32)>, ModError> {
            self.streams_requested.store(true, Ordering::SeqCst);
            Ok(Vec::new())
        }

        fn on_world_loaded(
            &mut self,
            _ctx: &mut mush_core::ModuleCtx<'_>,
            data: ModuleBootData,
        ) -> Result<(), ModError> {
            if data
                .kv
                .iter()
                .any(|(key, _)| is_action_state_persistence_key(key))
            {
                return Err(ModError::msg("reserved state leaked into module boot"));
            }
            self.loaded.store(true, Ordering::SeqCst);
            Ok(())
        }
    }

    fn state_boot_engine() -> (
        Engine,
        StateFamilyHandle<u64, BootState>,
        Arc<AtomicBool>,
        Arc<AtomicBool>,
    ) {
        let captured = Arc::new(Mutex::new(None));
        let streams_requested = Arc::new(AtomicBool::new(false));
        let loaded = Arc::new(AtomicBool::new(false));
        let engine = Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![Box::new(StateBootMod {
                handle: Arc::clone(&captured),
                streams_requested: Arc::clone(&streams_requested),
                loaded: Arc::clone(&loaded),
            })],
        )
        .unwrap();
        let handle = captured
            .lock()
            .expect("handle mutex poisoned")
            .clone()
            .expect("registration captured state handle");
        (engine, handle, streams_requested, loaded)
    }

    fn boot_state_op(key: &str, amount: u8) -> ModuleOp {
        ModuleOp {
            ns: "state_boot_test".into(),
            op: ModuleWrite::JsonSet {
                key: format!("action-state/v1/boot/{key}").into(),
                value: serde_json::json!({
                    "family": { "owner": "state_boot_test", "id": "boot" },
                    "key": key,
                    "schema": { "id": "boot_state.v1", "version": 1 },
                    "value": { "amount": amount },
                }),
            },
        }
    }

    /// Seed a module document through the same authoritative prefix owner that
    /// production uses. Reserved `action-state/*` fixtures must never bypass
    /// the store guard through the legacy module-write API.
    async fn seed_authoritative_json(
        store: &InMemoryStore,
        namespace: &str,
        key: &str,
        value: serde_json::Value,
    ) {
        let owner = OwnerId::new(namespace).unwrap();
        let mut definitions = DefinitionRegistryBuilder::new();
        let action = definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("action").unwrap(),
                    DefinitionId::new("boot_fixture").unwrap(),
                    1,
                    serde_json::json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let outcome = definitions
            .register(
                owner.clone(),
                DefinitionDescriptor::new(
                    DefinitionKind::new("outcome").unwrap(),
                    DefinitionId::new("boot_fixture_written").unwrap(),
                    1,
                    serde_json::json!({}),
                )
                .unwrap(),
            )
            .unwrap();
        let request = RequestUid::from_bytes([0xA1; 16]);
        let bundle = AuthoritativeBundle::new(AuthoritativeBundleDraft {
            token: CommitToken::from_bytes([0xA2; 16]),
            request,
            idempotency: ScopedIdempotencyKey::new(
                IdempotencyScope {
                    principal: PrincipalId::Object { id: GOD },
                    owner: owner.clone(),
                    namespace: CanonicalId::new("boot_fixture").unwrap(),
                },
                0,
                IdempotencyKey::Request(request),
            ),
            ingress_hash: CanonicalHash::from_bytes([0xA3; 32]),
            request_hash: CanonicalHash::from_bytes([0xA4; 32]),
            plan_hash: CanonicalHash::from_bytes([0xA5; 32]),
            action: action.clone(),
            provider: owner.clone(),
            cause: PreparedCausalRef::new(request, None, 0).unwrap(),
            definitions: vec![action.clone(), outcome.clone()],
            writes: vec![ReplayWrite::ModuleJsonSet {
                namespace: owner,
                key: AuthoritativeKey::parse_canonical(key).unwrap(),
                value,
            }],
            outcome: RulesValue::new(outcome.clone(), serde_json::json!({"seeded": true})).unwrap(),
            semantic_events: vec![
                mush_core::AuthoritativeSemanticEvent::root(
                    PreparedCausalRef::new(request, None, 0).unwrap(),
                    &action,
                    RulesValue::new(outcome, serde_json::json!({"seeded": true})).unwrap(),
                    vec![PrincipalId::Object { id: GOD }],
                )
                .unwrap(),
            ],
            eligible_witnesses: vec![PrincipalId::Object { id: GOD }],
            durable_outbox: vec![],
        })
        .unwrap();
        let token = bundle.token();
        let hash = bundle.bundle_hash();
        store.reserve_authoritative_intent(bundle).await.unwrap();
        store.reconcile_authoritative(token, hash).await.unwrap();
        store.checkpoint_authoritative(token, hash).await.unwrap();
    }

    async fn seed_authoritative_op(store: &InMemoryStore, op: ModuleOp) {
        let ModuleWrite::JsonSet { key, value } = op.op else {
            panic!("authoritative boot fixture must be a JSON set");
        };
        seed_authoritative_json(store, &op.ns, &key, value).await;
    }

    struct ReservedBootWrite;

    impl GameModule for ReservedBootWrite {
        fn name(&self) -> &'static str {
            "reserved_boot_write"
        }

        fn register(&mut self, _reg: &mut Registrar<'_>) -> Result<(), ModError> {
            Ok(())
        }

        fn on_world_loaded(
            &mut self,
            ctx: &mut mush_core::ModuleCtx<'_>,
            _data: ModuleBootData,
        ) -> Result<(), ModError> {
            ctx.persist(ModuleOp {
                ns: self.name().into(),
                op: ModuleWrite::JsonSet {
                    key: "action-state/v1/forged/1".into(),
                    value: serde_json::json!(true),
                },
            });
            Ok(())
        }
    }

    #[tokio::test]
    async fn module_boot_cannot_write_the_reserved_action_state_keyspace() {
        let store = InMemoryStore::new();
        let mut engine = Engine::with_modules(
            World::new_minimal(),
            mush_core::Registry::new(),
            vec![Box::new(ReservedBootWrite)],
        )
        .unwrap();

        let error = boot_modules(&mut engine, &store).await.unwrap_err();

        assert!(error.contains("coordinator-reserved"), "{error}");
        assert!(
            store
                .module_json_get("reserved_boot_write", "action-state/v1/forged/1")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn boot_hydrates_typed_state_before_module_callbacks() {
        let store = InMemoryStore::new();
        seed_authoritative_op(&store, boot_state_op("7", 3)).await;
        let (mut engine, handle, streams_requested, loaded) = state_boot_engine();

        boot_modules(&mut engine, &store).await.unwrap();

        assert!(streams_requested.load(Ordering::SeqCst));
        assert!(loaded.load(Ordering::SeqCst));
        assert_eq!(
            engine.action_state().get(&handle, &7).unwrap(),
            Some(&BootState { amount: 3 })
        );
    }

    #[tokio::test]
    async fn malformed_typed_state_aborts_before_any_module_callback() {
        let store = InMemoryStore::new();
        let mut op = boot_state_op("7", 3);
        let ModuleWrite::JsonSet { value, .. } = &mut op.op else {
            unreachable!()
        };
        value["schema"]["version"] = serde_json::json!(2);
        seed_authoritative_op(&store, op).await;
        let (mut engine, handle, streams_requested, loaded) = state_boot_engine();

        let error = boot_modules(&mut engine, &store).await.unwrap_err();

        assert!(error.contains("schema.version"), "{error}");
        assert!(!streams_requested.load(Ordering::SeqCst));
        assert!(!loaded.load(Ordering::SeqCst));
        assert!(engine.action_state().get(&handle, &7).unwrap().is_none());
    }

    #[tokio::test]
    async fn future_action_state_encoding_is_reserved_and_fails_closed() {
        let store = InMemoryStore::new();
        let mut op = boot_state_op("7", 3);
        let ModuleWrite::JsonSet { key, .. } = &mut op.op else {
            unreachable!()
        };
        *key = "action-state/v2/boot/7".into();
        seed_authoritative_op(&store, op).await;
        let (mut engine, _, streams_requested, loaded) = state_boot_engine();

        let error = boot_modules(&mut engine, &store).await.unwrap_err();

        assert!(error.contains("unsupported action-state"), "{error}");
        assert!(!streams_requested.load(Ordering::SeqCst));
        assert!(!loaded.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn fresh_namespace_is_stamped_without_running() {
        let store = InMemoryStore::new();
        let mut engine = toy_engine();
        boot_modules(&mut engine, &store).await.unwrap();
        let v = ns_version(&store, "toy").await.unwrap();
        assert_eq!(v, Some(2));
    }

    #[tokio::test]
    async fn preversioned_docs_migrate_and_stamp() {
        let store = InMemoryStore::new();
        // Seed a pre-versioning doc (no schema key).
        write_ops(
            &store,
            vec![ModuleOp {
                ns: "toy".into(),
                op: ModuleWrite::JsonSet {
                    key: "doc:1".into(),
                    value: serde_json::json!({ "a": 42 }),
                },
            }],
        )
        .await
        .unwrap();

        let mut engine = toy_engine();
        boot_modules(&mut engine, &store).await.unwrap();

        let doc = store
            .module_json_get("toy", "doc:1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc, serde_json::json!({ "b": 42 }));
        assert_eq!(ns_version(&store, "toy").await.unwrap(), Some(2));
    }

    #[tokio::test]
    async fn module_migrations_never_transform_reserved_action_state() {
        let store = InMemoryStore::new();
        let reserved = serde_json::json!({ "a": "must remain byte-equivalent" });
        seed_authoritative_json(&store, "toy", "action-state/v1/example/1", reserved.clone()).await;
        write_ops(
            &store,
            vec![ModuleOp {
                ns: "toy".into(),
                op: ModuleWrite::JsonSet {
                    key: "doc:1".into(),
                    value: serde_json::json!({ "a": 42 }),
                },
            }],
        )
        .await
        .unwrap();
        let engine = toy_engine();

        run_module_migrations(&store, engine.registry.migrations())
            .await
            .unwrap();

        assert_eq!(
            store
                .module_json_get("toy", "action-state/v1/example/1")
                .await
                .unwrap(),
            Some(reserved)
        );
        assert_eq!(
            store.module_json_get("toy", "doc:1").await.unwrap(),
            Some(serde_json::json!({ "b": 42 }))
        );
    }

    #[tokio::test]
    async fn newer_store_refuses_to_boot() {
        let store = InMemoryStore::new();
        write_ops(&store, vec![stamp("toy", 9)]).await.unwrap();
        let mut engine = toy_engine();
        let err = boot_modules(&mut engine, &store).await.unwrap_err();
        assert!(err.contains("newer than this build"), "{err}");
    }

    #[tokio::test]
    async fn schema_key_never_reaches_module_boot_data() {
        let store = InMemoryStore::new();
        write_ops(
            &store,
            vec![
                stamp("toy", 2),
                ModuleOp {
                    ns: "toy".into(),
                    op: ModuleWrite::JsonSet {
                        key: "doc:1".into(),
                        value: serde_json::json!({ "b": 1 }),
                    },
                },
            ],
        )
        .await
        .unwrap();
        let modules: Vec<Box<dyn GameModule>> = vec![Box::new(ToyMod { loaded: Vec::new() })];
        let boot = load_module_boot(&store, &modules).await.unwrap();
        let kv = &boot[&SmolStr::new("toy")].kv;
        assert_eq!(kv.len(), 1);
        assert_eq!(kv[0].0, "doc:1");
    }

    #[tokio::test]
    async fn action_state_keys_never_reach_module_boot_data() {
        let store = InMemoryStore::new();
        seed_authoritative_json(
            &store,
            "toy",
            "action-state/v99/future/1",
            serde_json::json!({ "opaque": true }),
        )
        .await;
        write_ops(
            &store,
            vec![
                stamp("toy", 2),
                ModuleOp {
                    ns: "toy".into(),
                    op: ModuleWrite::JsonSet {
                        key: "doc:1".into(),
                        value: serde_json::json!({ "b": 1 }),
                    },
                },
            ],
        )
        .await
        .unwrap();
        let modules: Vec<Box<dyn GameModule>> = vec![Box::new(ToyMod { loaded: Vec::new() })];

        let boot = load_module_boot(&store, &modules).await.unwrap();

        assert_eq!(boot[&SmolStr::new("toy")].kv.len(), 1);
        assert_eq!(boot[&SmolStr::new("toy")].kv[0].0, "doc:1");
    }

    #[tokio::test]
    async fn server_modules_boot_and_seed_durably() {
        // The real module list boots against an empty store; comsys'
        // first-boot seeding (the Public channel) must be written through.
        let store = InMemoryStore::new();
        let mut registry = crate::world_task::base_registry();
        let staff = std::sync::Arc::new(crate::staff::StaffQueue::new());
        crate::staff::register_server_commands(&mut registry, &staff).unwrap();
        let mut engine =
            Engine::with_modules(World::new_minimal(), registry, server_modules()).unwrap();
        boot_modules(&mut engine, &store).await.unwrap();

        let chans = store.module_json_list("comsys").await.unwrap();
        assert!(
            chans.iter().any(|(k, v)| k.starts_with("chan:")
                && v.get("name").and_then(serde_json::Value::as_str) == Some("Public")),
            "first boot must seed the Public channel durably: {chans:?}"
        );

        // A second boot over the same store must NOT re-seed (the member
        // docs and channel survive; God's membership from boot one holds).
        let mut registry = crate::world_task::base_registry();
        crate::staff::register_server_commands(&mut registry, &staff).unwrap();
        let mut engine2 =
            Engine::with_modules(World::new_minimal(), registry, server_modules()).unwrap();
        boot_modules(&mut engine2, &store).await.unwrap();
        let chans2 = store.module_json_list("comsys").await.unwrap();
        let count = |cs: &[(String, serde_json::Value)]| {
            cs.iter().filter(|(k, _)| k.starts_with("chan:")).count()
        };
        assert_eq!(count(&chans), count(&chans2), "reboot must not re-seed");
    }

    #[test]
    fn m5_modules_opt_into_the_testkit_cleanly() {
        // The testkit's `with_modules` is the module-authoring on-ramp: the
        // M5 scripting + weather modules must register (commands, hooks,
        // state slots), first-boot, and surface their seams there — the
        // "opt into the modules" confirmation for both.
        use mush_testkit::TestWorld;
        let tw = TestWorld::with_modules(vec![
            Box::new(mush_weather::WeatherModule::new()),
            Box::new(mush_script::ScriptModule::new()),
        ]);
        // Weather filled its reserved query seam...
        let (owner, _) = tw.registry().weather_query().expect("weather query seam");
        assert_eq!(owner.as_str(), "weather");
        // ...and script installed its post-command-table `$`-command
        // fallback (the object-command matcher).
        assert!(
            tw.registry().command_fallback_count() >= 1,
            "script registers a command fallback"
        );
        // Both modules are present under the one registry, no collision.
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert!(
            names.contains(&"weather") && names.contains(&"script"),
            "{names:?}"
        );
    }

    #[test]
    fn server_modules_register_without_collision() {
        // The full production list (including org/economy and sim/tactics) shares one
        // registry: registration must not collide, and every module must be present
        // in the load-bearing boot order.
        use mush_testkit::TestWorld;
        let tw = TestWorld::with_modules(server_modules());
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "comsys", "canon", "org", "faction", "scenes", "sim", "tactics", "economy", "mail",
                "jobs", "housing", "weather", "script"
            ]
        );
    }

    #[test]
    fn the_org_capability_vocabulary_covers_every_economy_capability() {
        // E3b2 §4.10: the wiring vocabulary is `mush_economy::CAPABILITIES` ∪
        // `mush_housing::CAPABILITIES` (E5b) ∪ the two org-administrative ids. If a
        // module starts asking about a capability the server does not wire,
        // `@org/grant` would refuse the very grant that module needs — so this is a
        // real coupling, pinned here.
        let vocab = org_capability_vocabulary();
        for c in mush_economy::CAPABILITIES.iter().chain(mush_housing::CAPABILITIES) {
            let id = mush_org::CapabilityId::new(c).expect("well-formed");
            assert!(vocab.contains(&id), "the wiring vocabulary omits {c}");
        }
        for c in ["org.role.manage", "org.grant.delegate"] {
            assert!(
                vocab.contains(&mush_org::CapabilityId::new(c).expect("well-formed")),
                "{c}"
            );
        }
        assert_eq!(
            vocab.len(),
            mush_economy::CAPABILITIES.len() + mush_housing::CAPABILITIES.len() + 2
        );
        assert!(
            !vocab.contains(
                &mush_org::CapabilityId::new("economy.acount.post").expect("well-formed")
            )
        );
    }

    #[test]
    fn ruleset_modules_selects_wod() {
        // "wod" resolves to the WoD ruleset module; the id survives boot.
        let mods = ruleset_modules(
            &["wod".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
        )
        .unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name(), "wod");
        // Case/whitespace-insensitive.
        let mods = ruleset_modules(
            &["  WoD ".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
        )
        .unwrap();
        assert_eq!(mods[0].name(), "wod");
        // Empty list is fine (a ruleset-less server).
        assert!(
            ruleset_modules(&[], &ruleset_wod::OnboardingConfig::default())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn ruleset_modules_selects_shadowrun() {
        // "shadowrun" (M10b) resolves to the SR5 ruleset module.
        let mods = ruleset_modules(
            &["shadowrun".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
        )
        .unwrap();
        assert_eq!(mods.len(), 1);
        assert_eq!(mods[0].name(), "shadowrun");
    }

    #[test]
    fn ruleset_modules_rejects_two_rulesets_at_once() {
        // Single-ruleset-per-server: enabling wod AND shadowrun (both register
        // +roll/+sheet/+chargen) is caught here with a clear message, not left
        // to surface as an opaque command collision at registration.
        let err = match ruleset_modules(
            &["wod".to_string(), "shadowrun".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("enabling two rulesets must be rejected"),
        };
        assert!(err.contains("one ruleset per server"), "{err}");
    }

    #[test]
    fn ruleset_modules_rejects_unknown_and_duplicate() {
        // `unwrap_err` needs `Debug` on the Ok value; `dyn GameModule` has
        // none, so match the error out by hand.
        let err = match ruleset_modules(
            &["tacsim".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("unknown ruleset must be rejected"),
        };
        assert!(err.contains("unknown ruleset"), "{err}");
        let err = match ruleset_modules(
            &["wod".to_string(), "wod".to_string()],
            &ruleset_wod::OnboardingConfig::default(),
        ) {
            Err(e) => e,
            Ok(_) => panic!("duplicate ruleset must be rejected"),
        };
        assert!(err.contains("more than once"), "{err}");
    }

    #[test]
    fn worldgen_registers_in_the_full_boot_list() {
        // main.rs's assembly: built-ins THEN worldgen THEN the config-selected
        // ruleset, sharing one registry. Worldgen registers with a
        // MockProvider-backed service (no network, no key); its @gen/@theme/@llm
        // commands and the worldgen.draft panel feed must all wire in cleanly.
        use mush_llm::{LlmService, MockProvider};
        use mush_testkit::TestWorld;

        let mock: Arc<dyn mush_llm::LlmProvider> = Arc::new(MockProvider::new("openrouter"));
        let llm = Arc::new(
            LlmService::builder()
                .provider("openrouter", mock)
                .global_enabled(false)
                .build(),
        );
        let mut mods = server_modules();
        mods.push(worldgen_module(Arc::clone(&llm)));
        mods.extend(
            ruleset_modules(
                &["wod".to_string()],
                &ruleset_wod::OnboardingConfig::default(),
            )
            .unwrap(),
        );
        let tw = TestWorld::with_modules(mods);
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "comsys", "canon", "org", "faction", "scenes", "sim", "tactics", "economy", "mail",
                "jobs", "housing", "weather", "script", "worldgen", "wod"
            ]
        );
        assert!(tw.registry().spec_of("@gen").is_some());
        assert!(tw.registry().spec_of("@theme").is_some());
        assert!(tw.registry().spec_of("@llm").is_some());
        assert!(
            tw.registry()
                .panel_feeds()
                .iter()
                .any(|(feed, _)| feed == "worldgen.draft")
        );
    }

    #[test]
    fn npc_module_registers_and_joins_the_panel_manifest() {
        // main.rs builds the web hello manifest from `registry.panel_feeds()`,
        // so registering `npc.thinking` is what puts the thinking indicator in
        // the manifest. This guards that (and that the npc module registers
        // cleanly alongside the built-ins). Dialogue-only grounding is enough.
        use mush_testkit::TestWorld;
        let mut mods = server_modules();
        mods.push(npc_module(None));
        let tw = TestWorld::with_modules(mods);
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert!(names.contains(&"npc"), "{names:?}");
        assert!(
            tw.registry()
                .panel_feeds()
                .iter()
                .any(|(feed, _)| feed == "npc.thinking"),
            "npc.thinking must be a registered panel feed (it joins the web manifest)"
        );
        // The @agent staff surface is live.
        assert!(tw.registry().spec_of("@agent").is_some());
    }

    #[test]
    fn ruleset_agent_grounds_a_shadowrun_npc_end_to_end() {
        // The generic grounding adapter carries the API-scaling claim end-to-end:
        // on a shadowrun-only world, an NPC grounds through the Ruleset trait
        // (sheet bootstrap + capability profile), not the concrete WodAgent.
        use mush_npc::AgentGrounding;
        use mush_testkit::TestWorld;
        let mut mods = server_modules(); // carries the jobs module the ruleset needs
        mods.extend(
            ruleset_modules(
                &["shadowrun".to_string()],
                &ruleset_wod::OnboardingConfig::default(),
            )
            .unwrap(),
        );
        let mut tw = TestWorld::with_modules(mods);
        let npc = tw.spawn_player("Ganger");
        let agent = RulesetAgent::new(ruleset_shadowrun::ShadowrunRuleset::new());
        // No sheet yet → dialogue-only (no profile).
        assert!(
            agent.profile(tw.world(), npc).is_none(),
            "an NPC with no sheet does not ground"
        );
        // The `@agent/sheet` bootstrap gives it a real SR5 sheet via Ruleset::init_sheet.
        agent
            .init_sheet(tw.world_mut(), npc, "", npc)
            .expect("SR5 init_sheet writes a sheet");
        // Now it grounds: a capability profile carrying the sr_roll tool.
        let prof = agent
            .profile(tw.world(), npc)
            .expect("SR5 NPC grounds through the generic Ruleset seam after init_sheet");
        assert!(
            prof.tools.iter().any(|t| t.name == "sr_roll"),
            "the grounded SR5 NPC exposes the sr_roll ability: {:?}",
            prof.tools
        );
    }

    #[test]
    fn npc_agent_adapter_grounds_every_enabled_ruleset() {
        // The generic grounding adapter is built for any ruleset that grounds
        // NPCs — wod AND shadowrun (each dispatches through its Ruleset trait
        // impl). A ruleset-less config, or one whose ruleset has no grounding,
        // runs NPCs dialogue-only (None).
        assert!(npc_agent_adapter(&["wod".to_string()]).is_some());
        assert!(npc_agent_adapter(&["  WoD ".to_string()]).is_some());
        assert!(
            npc_agent_adapter(&["shadowrun".to_string()]).is_some(),
            "SR5 NPCs must ground through the generic Ruleset seam, not just WoD"
        );
        assert!(npc_agent_adapter(&["  Shadowrun ".to_string()]).is_some());
        assert!(npc_agent_adapter(&[]).is_none());
        assert!(npc_agent_adapter(&["tacsim".to_string()]).is_none());
    }

    #[test]
    fn full_boot_list_registers_without_collision() {
        // The production concatenation main.rs builds — built-ins THEN the
        // config-selected wod ruleset — shares one registry with no
        // collision, and wod registers last (after jobs, so its approval
        // payload handlers see a loaded jobs module).
        use mush_testkit::TestWorld;
        let mut mods = server_modules();
        mods.extend(
            ruleset_modules(
                &["wod".to_string()],
                &ruleset_wod::OnboardingConfig::default(),
            )
            .unwrap(),
        );
        let tw = TestWorld::with_modules(mods);
        let names: Vec<&str> = tw.engine().modules().iter().map(|m| m.name()).collect();
        assert_eq!(
            names,
            [
                "comsys", "canon", "org", "faction", "scenes", "sim", "tactics", "economy", "mail",
                "jobs", "housing", "weather", "script", "wod"
            ]
        );
        // The ruleset's surface is live: +roll / +sheet registered, the
        // wod.roll panel feed present, and the chargen/xp job queues too.
        assert!(tw.registry().spec_of("+roll").is_some());
        assert!(tw.registry().spec_of("+sheet").is_some());
        assert!(
            tw.registry()
                .panel_feeds()
                .iter()
                .any(|(feed, _)| feed == "wod.roll")
        );
    }
}

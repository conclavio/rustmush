//! The all-in-one composition root: everything `mushd` builds before it binds
//! a listener.
//!
//! Split out of the binary's `serve` so the composition the deployed all-in-one
//! runs is the composition the integration suite can boot and assert on — every
//! `crates/mushd/tests` case rides this exact roster. WS04-S0 closed the four
//! omissions this root had drifted into against the forked server half's
//! `build_engine`: the durable decision key ring, the boot-hydrated account
//! oracle, the `[tacsim]` pulse cadence, and a validated
//! `[store].receipt_policy`.
//!
//! WS04-S1 then took the store open itself out of here: `open_store` (the
//! receipt-policy gate AND the J5 archive tier) lives once, in
//! [`mush_server::boot`], and both roots call it.

use std::sync::Arc;

use mush_config::config::Config;
use mush_config::decision_keys::DurableDecisionKeyStore;
use mush_config::offer_keys::DurableOfferKeyStore;
use mush_config::resolution_keys::DurableResolutionKeyStore;
use mush_core::Engine;
use mush_llm::LlmService;
use mush_server::config_ext::{
    atlas_config, economy_config_with, llm_config, onboarding_ruleset, shadowrun_cadences,
    shadowrun_economy_config, tacsim_config, weather_config, wod_economy_config, worldgen_config,
};
use mush_store::{LedgerArchive, ObjectStore, core_migrations, run_migrations};

use crate::accounts::import_accounts_json;
use crate::persist::{load_or_seed_world, reconcile_authoritative_for_writable_boot};
use crate::quarantine::QuarantineSpool;
use crate::staff::{StaffQueue, register_server_commands_with_cap};

/// Everything [`boot_all_in_one`] hands the serving half of `mushd`.
pub struct Booted {
    /// The authoritative store, already migrated and writable.
    pub store: Arc<dyn ObjectStore>,
    /// The J5 ledger archive tier, when `[store.archive].enabled`.
    pub ledger_archive: Option<Arc<LedgerArchive>>,
    /// The composed, module-booted engine, not yet handed to the world task.
    pub engine: Engine,
    /// The staff queue the server commands push into and the world loop drains.
    pub staff: Arc<StaffQueue>,
    /// The shared LLM service handle.
    pub llm: Arc<LlmService>,
    /// The validated `[llm]` config (the world task reads `decision_workers`).
    pub llm_config: mush_llm::LlmConfig,
}

/// Boot the all-in-one world: quarantine barrier, durable key rings, store
/// open + migrations + account import, the module roster, the world load, and
/// every module's boot pass — everything before a listener exists.
///
/// The caller keeps the [`QuarantineSpool`] (it moves on into `ServerDeps`),
/// so it is borrowed here rather than consumed.
///
/// # Errors
///
/// Any boot step that refuses: a failed quarantine barrier, unprovisionable key
/// rings, an unreachable or non-durable store, a rejected config stanza, a
/// module registration clash, or a module boot pass that fails.
pub async fn boot_all_in_one(
    config: &Config,
    quarantine_spool: &mut QuarantineSpool,
) -> Result<Booted, String> {
    // The pre-opened spool still owns the single-deployment lock. Compare any
    // valid active records through the store's read-only connection before
    // migrations, account import, world seeding, or module boot can write.
    mush_server::prepare_quarantine_for_writable_boot(config, quarantine_spool, None).await?;
    let offer_keys = DurableOfferKeyStore::new(config.offer_key_dir())
        .load_or_create()
        .map_err(|error| format!("offer key provisioning failed: {error}"))?;
    tracing::info!(
        current_version = offer_keys.current_version(),
        key_dir = %config.offer_key_dir().display(),
        "durable legal-action offer keys loaded"
    );
    let resolution_keys = DurableResolutionKeyStore::new(config.resolution_key_dir())
        .load_or_create()
        .map_err(|error| format!("resolution key provisioning failed: {error}"))?;
    tracing::info!(
        current_version = resolution_keys.current_version(),
        key_dir = %config.resolution_key_dir().display(),
        "durable authoritative resolution keys loaded"
    );
    let decision_keys = DurableDecisionKeyStore::new(config.decision_key_dir())
        .load_or_create()
        .map_err(|error| format!("decision key provisioning failed: {error}"))?;
    tracing::info!(
        current_version = decision_keys.current_version(),
        key_dir = %config.decision_key_dir().display(),
        "durable delayed-decision keys loaded"
    );
    // Store: connect (fail fast), migrate, one-time accounts.json import,
    // then load or seed the world — all before anything serves.
    // WS04-S1: one `open_store` for both roots — the receipt-policy gate and
    // the J5 archive tier live in the server half's boot root, and the
    // all-in-one calls it rather than carrying a second transcription.
    let mush_server::boot::OpenedStore {
        store,
        archive: ledger_archive,
    } = mush_server::boot::open_store(config).await?;
    let authoritative = store.authoritative_store().ok_or_else(|| {
        "configured store does not implement K3 exact-token durability; refusing writable boot"
            .to_owned()
    })?;
    let recovery = reconcile_authoritative_for_writable_boot(authoritative).await?;
    tracing::info!(
        indexed = recovery.indexed,
        retained = recovery.retained,
        reconciled = recovery.reconciled,
        already_finalized = recovery.already_finalized,
        checkpointed = recovery.checkpointed,
        checkpoint_deferred_outbox = recovery.checkpoint_deferred_outbox,
        scan_pages = recovery.scan_pages,
        "authoritative intent recovery barrier complete"
    );
    store
        .bootstrap_writable()
        .await
        .map_err(|error| format!("store writable bootstrap failed: {error}"))?;
    let version = run_migrations(&*store, &core_migrations())
        .await
        .map_err(|e| format!("store migrations failed: {e}"))?;
    tracing::info!(schema = version, "store schema current");
    let imported = import_accounts_json(&*store, &config.server.data_dir).await?;
    if imported > 0 {
        tracing::info!(imported, "M1 accounts.json imported");
    }
    // E7c5 B1 (WS04-S0): build the account-distinctness oracle from the store,
    // once, before composition (the store is in scope and async here). A legacy
    // account with no stored id is minted on the fly (deterministic); a
    // character claimed by two accounts is left unmapped and paged.
    let account_identity = mush_server::accounts_identity::hydrate(&*store).await?;
    tracing::info!(
        mapped_characters = account_identity.len(),
        "M-E7c5 account oracle hydrated"
    );
    // Command table: core + engine + server staff commands (@siteban/@dump/
    // @boot/@ps/@stats push typed ops onto the queue the world task
    // drains), plus the markdown help topics.
    let staff = Arc::new(StaffQueue::new());
    let mut registry = crate::world_task::base_registry();
    register_server_commands_with_cap(&mut registry, &staff, config.game.max_characters as usize)
        .map_err(|e| format!("server command registration: {e}"))?;
    match crate::help::load_help_topics(&mut registry, &config.server.help_dir) {
        Ok(n) => tracing::info!(topics = n, "help topics loaded"),
        Err(e) => tracing::warn!(error = %e, dir = %config.server.help_dir.display(),
            "help topics failed to load; continuing without them"),
    }
    // M7 LLM service: built from [llm] config, resolving each provider's API
    // key from its named env var (never inlined in config, never logged). A
    // keyless clone still boots — `enabled` defaults false, and a provider
    // whose key env-var is unset is skipped (routes to it fail fast). The
    // worldgen module and the world task's `Effect::ModuleAsync` driver share
    // this one handle.
    let llm_cfg = llm_config(&config.llm)?;
    let llm = Arc::new(LlmService::from_config(&llm_cfg).map_err(|e| format!("llm config: {e}"))?);
    tracing::info!(
        enabled = llm_cfg.enabled,
        providers = llm_cfg.providers.len(),
        routes = llm_cfg.routes.len(),
        "llm service built"
    );

    // Game modules: the built-ins (including sim + tactics), then
    // the M7 worldgen module, then the config-selected rulesets
    // ([rulesets].enabled — M6: "wod"). Everything after the built-ins
    // registers cleanly against the one registry; rulesets go last so their
    // approval payload handlers see a loaded jobs module. Then run each
    // namespace's store migrations and hand each module its persisted boot
    // data — all before the world task starts serving.
    let onboarding = onboarding_ruleset(&config.onboarding);
    // E5c: `[wod.economy]`, validated here (an unknown mode is a hard boot error).
    // `None` when no WoD ruleset is enabled — that is what gates the housing title
    // issuer (08d §2).
    let wod_economy = wod_economy_config(&config.wod)?;
    let wod_selected = config
        .rulesets
        .enabled
        .iter()
        .any(|r| r.trim().eq_ignore_ascii_case("wod"));
    // E6a: `[shadowrun.economy]`, validated here (an unknown lifestyle tier is a hard
    // boot error). `None` when no Shadowrun ruleset is enabled — that is what gates
    // the run board's economy observer (08f §2's `issuers` pattern).
    let sr_economy = shadowrun_economy_config(&config.shadowrun)?;
    let sr_selected = config
        .rulesets
        .enabled
        .iter()
        .any(|r| r.trim().eq_ignore_ascii_case("shadowrun"));
    // SP1 / D-LBK-7: a Shadowrun server needs `sr.month` declared or no lifestyle
    // obligation can exist. The host contributes the stanza unless the operator
    // declared one of that name themselves.
    let contributed: Vec<(String, mush_server::config_ext::DeclaredCadence)> = if sr_selected {
        shadowrun_cadences(&sr_economy)
    } else {
        Vec::new()
    };
    let contributed: Vec<(&str, mush_server::config_ext::DeclaredCadence)> = contributed
        .iter()
        .map(|(n, d)| (n.as_str(), d.clone()))
        .collect();
    // E7 cut 1: the TacSim layer's frozen block, read **host-side** so an unreadable,
    // oversized or digest-mismatched pack fails the boot naming the file before any
    // module runs (invariant (iv)). E7 residue pass 1 (R4) reads it here, before the
    // module list is built, because `[tacsim.recovery]` reaches `mush-campaign` —
    // which composes inside `server_modules_with` — through the composition root's
    // own mapping and never through a crate edge.
    let tacsim = tacsim_config(&config.tacsim)?;
    // S1: `[atlas]`, read host-side for the same reason as `[tacsim]`.
    let atlas = atlas_config(&config.atlas)?;
    let mut mods = crate::modules::server_modules_with(
        weather_config(&config.weather),
        economy_config_with(&config.economy, &contributed)?,
        atlas,
        &onboarding,
        wod_selected.then_some(&wod_economy),
        sr_selected.then_some(&sr_economy),
        crate::modules::recovery_policy(&tacsim.recovery),
    );
    mods.push(crate::modules::worldgen_module_with(
        Arc::clone(&llm),
        worldgen_config(&config.worldgen),
    ));
    // M8: the NPC agent module, grounded by the enabled ruleset's sheet +
    // on-loop abilities through the generic ruleset grounding adapter (WoD,
    // Shadowrun, …); dialogue-only when no ruleset is enabled.
    let npc_grounding = crate::modules::npc_agent_adapter(&config.rulesets.enabled);
    mods.push(crate::modules::npc_module_with(npc_grounding, &config.npc));
    let rulesets = crate::modules::ruleset_modules(
        &config.rulesets.enabled,
        &onboarding,
        &wod_economy,
        &sr_economy,
    )
    .map_err(|e| format!("ruleset selection: {e}"))?;
    if !rulesets.is_empty() {
        tracing::info!(rulesets = ?config.rulesets.enabled, "rulesets enabled");
    }
    mods.extend(rulesets);
    // E7 cut 1: the TacSim layer, **after** the character ruleset (doc 10 §20, 10b
    // §3.1). It is not a `[rulesets]` id — `[tacsim].enabled` switches it, independently
    // of which single character ruleset is loaded — and it composes last so its boot
    // can read that ruleset's facts. Its config was read above, before the module list,
    // for `[tacsim.recovery]`'s sake.
    if tacsim.enabled {
        tracing::info!(
            packs = tacsim.packs.len(),
            seeds = tacsim.seeds.len(),
            "tacsim enabled"
        );
    }
    mods.push(crate::modules::tacsim_module(tacsim));
    // S1: the content module LAST — see `mush_server::modules::content_module`.
    mods.push(crate::modules::content_module(config.game.starter_area)?);
    // Compose and freeze every mechanics registry before persisted world data
    // is loaded. This makes the C1 boot order structural instead of relying on
    // individual modules to avoid early interpretation.
    let composition =
        Engine::compose_modules(registry, mods).map_err(|e| format!("module registration: {e}"))?;
    let mut world = load_or_seed_world(&*store).await?;
    // Hydrate the versioned IC timeline before module state and ticks are
    // materialized into the engine. [game] seeds an empty store only;
    // persisted epochs remain authoritative across configuration changes.
    let clock = config.game.game_clock().map_err(|e| e.to_string())?;
    mush_server::timeline::hydrate_world_timeline(&mut world, &*store, clock).await?;
    tracing::info!(
        generation = world.world_timeline().current().generation,
        ic_rate = world.world_timeline().current().rate,
        "world timeline installed",
    );
    let mut engine = Engine::from_composition(world, composition);
    engine.set_offer_keys(offer_keys);
    engine.set_resolution_keys(resolution_keys);
    engine.set_decision_keys(decision_keys);
    // E7c5 B1: upgrade the sim's account oracle (installed unwired at register)
    // with the boot-hydrated map, so cut 5's `+focus` consults it synchronously
    // inside a world command. If the sim did not compose the slot the oracle
    // stays absent and every consumer fails closed.
    let account_identity: Arc<dyn mush_core::AccountIdentity> = Arc::new(account_identity);
    // Programme D4: the relay's `(account, actor)` re-validation reads the
    // same oracle from the engine.
    engine.set_account_identity(Arc::clone(&account_identity));
    if let Some(slot) = engine.world.module_state_mut::<mush_sim::AccountOracle>() {
        *slot = mush_sim::AccountOracle::new(account_identity);
    }
    // E7c8 B1: upgrade the pulse cadence from `[tacsim]`, on the same
    // post-composition device — §7 lists `ground_pulse_ms`/`space_pulse_ms` as
    // "parsed and inert until cut 8", and this is the cut that reads them. A
    // value outside the constructible band is CLAMPED, not refused: a knob that
    // configures a cadence must not be able to stop a world booting, and the
    // clamp is visible in `@domain/status`.
    if let Some(slot) = engine
        .world
        .module_state_mut::<mush_sim::tick::PulseConfig>()
    {
        *slot = mush_sim::tick::PulseConfig {
            ground_ms: config.tacsim.ground_pulse_ms,
            space_ms: config.tacsim.space_pulse_ms,
        };
    }
    crate::modules::boot_modules(&mut engine, &*store).await?;
    Ok(Booted {
        store,
        ledger_archive,
        engine,
        staff,
        llm,
        llm_config: llm_cfg,
    })
}

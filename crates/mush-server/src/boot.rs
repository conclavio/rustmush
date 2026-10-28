//! The boot root both deployments share: open the store, compose and boot the
//! engine, schedule the J5 ledger archive pass, and describe the booted world
//! to the gateway.
//!
//! Before WS04-S1 each half owned half of this. `mush-server` validated
//! `[store].receipt_policy` and never opened a [`LedgerArchive`]; `mushd`
//! opened the archive, attached it to both backends and scheduled the nightly
//! pass, and (until WS04-S0) never read the receipt policy at all. A knob
//! wired into one root only is a knob that silently does nothing on the other,
//! and the fork is meant to run the same world twice — so both live here, once,
//! and both roots call it.
//!
//! What this module owns:
//!
//! - [`open_store`] — the backend, the receipt-policy gate, and the archive
//!   tier attached to whichever backend was chosen, so `log:` reads fall
//!   through to it on both halves. The Redis failure carries `mushd`'s
//!   operator remediation text (start Redis, repoint `[store].redis_url`, or
//!   say `[store].store = "memory"`), which is the richer of the two messages
//!   the fork had drifted into and the one dev1/dev2 boot on.
//! - [`spawn_ledger_archive`] — the write/purge half of the tier. It RETURNS
//!   its [`JoinHandle`] rather than detaching it, so the caller can bind it
//!   into a shutdown tail; a dropped handle is the mushd-5 shape this
//!   workstream exists to abolish. [`stop_ledger_archive`] is the other end of
//!   that binding.
//! - [`game_modules`] — the roster, and the ONE place its order is stated.
//! - [`build_engine`] — the whole world boot behind that roster: the three key
//!   rings, the K3 recovery barrier, the writable bootstrap, migrations, the
//!   world load, and every module's boot pass.
//! - [`build_hello_manifest`] — the features/panels list the gateway turns
//!   into the web client's `hello` frame.
//!
//! # The caller-supplied-store branch
//!
//! [`serve_with_store`](crate::serve_with_store) and
//! [`serve_with_quarantine_spool`](crate::serve_with_quarantine_spool) accept a
//! store the CALLER opened (the restart-survival rig passes one store across
//! every reboot). That branch bypasses [`open_store`] entirely, so it composes
//! no archive tier and schedules no nightly pass: with
//! `[store.archive].enabled = true` in the config, the knob does nothing. That
//! is a legitimate arrangement — a caller who brought their own store owns its
//! tiering — but it is exactly the "wired into one path only" shape this
//! workstream abolishes, so the branch says so out loud in a `warn!` rather
//! than disarming the tier in silence.

use std::sync::Arc;

use mush_config::accounts::import_accounts_json;
use mush_config::config::{Config, StoreBackend, StoreReceiptPolicy};
use mush_config::decision_keys::DurableDecisionKeyStore;
use mush_config::geology_keys::DurableGeologyKeyStore;
use mush_config::metrics::Metrics;
use mush_config::offer_keys::DurableOfferKeyStore;
use mush_config::resolution_keys::DurableResolutionKeyStore;
use mush_core::{Engine, GameModule};
use mush_llm::LlmService;
use mush_store::{
    InMemoryStore, LedgerArchive, ObjectStore, RedisStore, core_migrations, run_migrations,
};
use tokio::task::JoinHandle;

use crate::archive::{ArchiveConfig, spawn_archive_job};
use crate::persist::{load_or_seed_world, reconcile_authoritative_for_writable_boot};
use crate::quarantine::QuarantineSpool;
use crate::staff::{StaffQueue, register_server_commands_with_cap};
use crate::{config_ext, modules, timeline, world_task};

/// What [`open_store`] hands back: the authoritative store, and the archive
/// tier beside it when `[store.archive].enabled`.
///
/// The archive is already attached to `store` (reads fall through to it); it is
/// returned separately because the nightly WRITE pass needs its own handle —
/// see [`spawn_ledger_archive`].
pub struct OpenedStore {
    /// The configured backend, archive-attached when the tier is enabled.
    pub store: Arc<dyn ObjectStore>,
    /// The J5 ledger archive tier, when `[store.archive].enabled`.
    pub archive: Option<Arc<LedgerArchive>>,
}

/// Neither field is `Debug` (an `Arc<dyn ObjectStore>` cannot be), and a boot
/// error is far more useful than the store's innards, so this reports only what
/// was opened — enough for `Result::expect_err` to name the shape it did not
/// get.
impl std::fmt::Debug for OpenedStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpenedStore")
            .field("archive", &self.archive.is_some())
            .finish_non_exhaustive()
    }
}

/// Open the configured store backend, failing fast with an operator-facing
/// error when Redis is unreachable (PLAN: never run without persistence).
///
/// `[store].receipt_policy` is validated here and nowhere else: a spelling the
/// parser does not know refuses the boot naming the value rather than quietly
/// downgrading to `applied`, and `host_power_loss` on the memory backend is
/// refused outright (nothing in RAM survives a power cut, so the promise is
/// unkeepable). On Redis the policy is additionally proved against the live
/// server before the store is handed out.
///
/// With `[store.archive].enabled`, the ledger archive tier (J5) is opened under
/// `[server].data_dir` and attached to whichever backend was chosen, so a
/// purged `log:` chunk still answers through the store.
///
/// # Errors
///
/// An unparseable `[store]` stanza, an unreachable Redis, a Redis whose
/// durability does not satisfy the receipt policy, `host_power_loss` on the
/// memory backend, or an unopenable archive directory.
pub async fn open_store(config: &Config) -> Result<OpenedStore, String> {
    let receipt_policy = config.store_receipt_policy().map_err(|e| e.to_string())?;
    let archive = if config.store.archive.enabled {
        let root = config.server.data_dir.join(&config.store.archive.dir);
        let archive = LedgerArchive::open(&root)
            .map_err(|e| format!("opening the ledger archive at {}: {e}", root.display()))?;
        let stats = archive.stats();
        tracing::info!(
            root = %root.display(),
            chunks = stats.chunks,
            purged = stats.purged,
            compressed_mb = stats.compressed_bytes / (1024 * 1024),
            "ledger archive tier attached"
        );
        Some(Arc::new(archive))
    } else {
        None
    };
    let store: Arc<dyn ObjectStore> = match config.store_backend().map_err(|e| e.to_string())? {
        StoreBackend::Redis => {
            let url = &config.store.redis_url;
            tracing::info!(%url, "connecting to Redis");
            // The remediation text is deliberately long: this is the single
            // most common first-run failure, and the terse `cannot reach Redis`
            // the split half used to print left the operator guessing.
            let store = RedisStore::connect_read_only(url).await.map_err(|e| {
                format!(
                    "cannot reach Redis at {url}: {e}\n\
                     mushd requires its store at boot. Start Redis (or point \
                     [store].redis_url at it), or set [store].store = \"memory\" \
                     for a throwaway world."
                )
            })?;
            if receipt_policy == StoreReceiptPolicy::HostPowerLoss {
                store
                    .require_host_power_loss_receipts()
                    .await
                    .map_err(|e| format!("Redis does not satisfy [store].receipt_policy: {e}"))?;
            }
            match &archive {
                Some(archive) => Arc::new(store.with_archive(Arc::clone(archive))),
                None => Arc::new(store),
            }
        }
        StoreBackend::Memory => {
            if receipt_policy == StoreReceiptPolicy::HostPowerLoss {
                return Err(
                    "[store].receipt_policy=\"host_power_loss\" requires the Redis backend"
                        .to_owned(),
                );
            }
            tracing::warn!("[store].store = \"memory\": nothing will survive a restart");
            match &archive {
                Some(archive) => Arc::new(InMemoryStore::new().with_archive(Arc::clone(archive))),
                None => Arc::new(InMemoryStore::new()),
            }
        }
    };
    Ok(OpenedStore { store, archive })
}

/// Schedule the J5 nightly archive pass — the WRITE/purge half of the tier —
/// and hand the caller its [`JoinHandle`].
///
/// `archive` is [`OpenedStore::archive`]; `None` (the tier is off) is `Ok(None)`
/// and not an error. `namespaces` is every composed module's namespace, which
/// only the booted engine knows.
///
/// The handle is RETURNED rather than detached on purpose: the job runs beside
/// the store writer for the life of the process, and a caller that drops it has
/// no way to abort it or to notice it ended. Bind it into the shutdown tail.
///
/// # Errors
///
/// `[store.archive].window` is not two `HH:MM` times joined by `-`, or names a
/// time out of range. That is a config error, so it refuses the boot instead of
/// leaving the pass silently unscheduled for the life of the deployment.
pub fn spawn_ledger_archive(
    config: &Config,
    store: &Arc<dyn ObjectStore>,
    archive: Option<Arc<LedgerArchive>>,
    namespaces: Vec<String>,
    metrics: &Arc<Metrics>,
) -> Result<Option<JoinHandle<()>>, String> {
    let Some(archive) = archive else {
        return Ok(None);
    };
    let cfg = &config.store.archive;
    let (window_start_min, window_end_min) = ArchiveConfig::parse_window(&cfg.window)?;
    let job = ArchiveConfig {
        window_start_min,
        window_end_min,
        retain_hours: cfg.retain_hours,
        high_watermark_bytes: cfg.high_watermark_mb.saturating_mul(1024 * 1024),
        low_watermark_bytes: cfg.low_watermark_mb.saturating_mul(1024 * 1024),
        segment_chunks: cfg.segment_chunks,
        max_purge_per_pass: cfg.max_purge_per_pass,
        namespaces,
    };
    tracing::info!(
        window = %cfg.window,
        retain_hours = cfg.retain_hours,
        high_watermark_mb = cfg.high_watermark_mb,
        low_watermark_mb = cfg.low_watermark_mb,
        "ledger archive pass scheduled"
    );
    Ok(Some(spawn_archive_job(
        Arc::clone(store),
        archive,
        job,
        Arc::clone(metrics),
    )))
}

/// Stop the nightly archive pass and WAIT for the task to end.
///
/// The other end of [`spawn_ledger_archive`]'s returned handle. The pass has no
/// shutdown seam of its own — it loops on a one-minute sleep for the life of the
/// process — so the only way to end it is to cancel it and then join it, and the
/// join is the point: a caller that merely dropped the handle would return with
/// the task still live inside a runtime it is about to tear down, which is how a
/// mid-pass copy becomes a torn one nobody waited for.
///
/// Cancellation mid-pass is safe: a pass copies chunks into the archive and only
/// then purges the primary, and both halves are idempotent, so the worst a
/// cancelled pass leaves behind is chunks the next night's pass copies again.
///
/// `None` (the tier is off) is a no-op.
pub async fn stop_ledger_archive(job: Option<JoinHandle<()>>) {
    let Some(job) = job else {
        return;
    };
    job.abort();
    match job.await {
        Ok(()) => tracing::info!("ledger archive pass ended"),
        Err(error) if error.is_cancelled() => {
            tracing::info!("ledger archive pass cancelled at shutdown");
        }
        // A panicked pass is worth naming: nothing has been purged since it died.
        Err(error) => tracing::error!(%error, "ledger archive pass ended badly"),
    }
}

/// The game module roster, in composition order — **the one place that order is
/// stated**.
///
/// Both roots build the same world, so both build it from here. The order is
/// registration order, which is the order hooks and observers fire in and the
/// order `on_world_loaded` runs in, so it is load-bearing:
///
/// 1. the built-ins ([`modules::server_modules_with_tacsim`]) first, with org
///    before economy and sim before economy — see that function — and (doc 18
///    cut IC5, D-18-33) `tacsim` named `mush-inventory`'s third quantity
///    delegate, returning the inventory cell the layer is handed;
/// 2. the M7 worldgen module after the built-ins;
/// 3. the NPC agent module, grounded by the pilot sheet through the generic
///    grounding adapter;
/// 4. the unconditional [`modules::sheet_modules`] next, so their approval
///    payload handlers see a loaded jobs module;
/// 5. the TacSim layer, switched by `[tacsim].enabled`, after the sheet so its
///    boot can read the pilot facts through the generic seam;
/// 6. the content module last, so its first-boot packs see every applier.
///
/// `[tacsim]` is read FIRST, before the list is built, for two reasons. E7 cut 1:
/// an unreadable, oversized or digest-mismatched pack must fail the boot naming
/// the file before any module runs (10b §3.1 invariant (iv)), and only the host
/// can fail a boot. E7 residue pass 1 (R4): `[tacsim.recovery]` reaches
/// `mush-campaign` — which composes INSIDE [`modules::server_modules_with_tacsim`] —
/// through this root's own mapping and never through a crate edge, so the policy
/// has to exist before the built-ins are built. The split half passed
/// `RecoveryConfig::default()` there until WS04-S2, and composed no TacSim layer
/// at all, so it carried a dead knob beside a missing one.
///
/// # Errors
///
/// A rejected `[tacsim]`, `[economy]` or `[atlas]` stanza, or a pack file that
/// will not verify — each naming what it refused.
pub fn game_modules(
    config: &Config,
    llm: &Arc<LlmService>,
) -> Result<Vec<Box<dyn GameModule>>, String> {
    game_modules_with_optional_commerce(config, llm, None)
}

/// The existing production roster with a host-supplied, opaque commerce policy.
/// Default boot never selects this API. It preserves owner order and all boot
/// hooks; Initialize and Active still need the real durable setup/audits.
///
/// # Errors
/// Ordinary configuration errors, or a Tacsim config/catalog that differs from
/// the exact resources captured by `commerce`.
pub fn game_modules_with_mind_commerce(
    config: &Config,
    llm: &Arc<LlmService>,
    commerce: &modules::mind_commerce::MindCommerce,
) -> Result<Vec<Box<dyn GameModule>>, String> {
    game_modules_with_optional_commerce(config, llm, Some(commerce))
}

/// Shared host-only preflight. World owner identities and readiness are checked
/// later by actual composition; this never opens the store or creates owners.
fn load_launch_deployment(
    config: &Config,
) -> Result<Option<tacsim::launch::LaunchDeployment>, String> {
    use std::io::Read;
    let Some(path) = config.tacsim.minds.launch_path.as_ref() else {
        return Ok(None);
    };
    if !config.tacsim.enabled {
        return Err("launch deployment requires enabled Tacsim".to_owned());
    }
    if path.is_empty() {
        return Err("launch deployment path is empty".to_owned());
    }
    let file = std::fs::File::open(path).map_err(|e| format!("launch deployment {path}: {e}"))?;
    let mut bytes = Vec::new();
    file.take((tacsim::launch::MAX_DEPLOYMENT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("launch deployment {path}: {e}"))?;
    tacsim::launch::LaunchDeployment::decode(&bytes)
        .map(Some)
        .map_err(|e| format!("launch deployment {path}: {e}"))
}

fn game_modules_with_optional_commerce(
    config: &Config,
    llm: &Arc<LlmService>,
    commerce: Option<&modules::mind_commerce::MindCommerce>,
) -> Result<Vec<Box<dyn GameModule>>, String> {
    // E7 cut 1: the TacSim layer's frozen block, read **host-side** and BEFORE the
    // module list, because `[tacsim.recovery]` is mapped into the built-ins below.
    let tacsim = config_ext::tacsim_config(&config.tacsim)?;
    if commerce.is_some() && config.tacsim.minds.launch_path.is_some() {
        return Err("choose one explicit mind commerce composition".into());
    }
    let launch = load_launch_deployment(config)?
        .map(|deployment| {
            let layer = tacsim::TacsimRuleset::with_game_content(tacsim.clone(), tacsim.enabled)
                .with_conduct_carrier()
                .map_err(|e| e.to_string())?;
            tacsim::launch::LaunchComposition::prepare(&layer, deployment)
                .map_err(|e| e.to_string())
        })
        .transpose()?;
    // S1: `[atlas]` likewise — its `pack_paths` are read host-side so a bad pack file
    // fails the boot naming the file, and the typed config reaches the atlas inside
    // the built-ins.
    let atlas = config_ext::atlas_config(&config.atlas, &config.worldgen)?;
    let onboarding = config_ext::onboarding_config(&config.onboarding);
    // 16b cut R-b: `[tacsim.economy]`, validated here (a bad instrument id is a hard
    // boot error). It decides the housing and market title issuers.
    let game = config_ext::tacsim_economy_config(&config.tacsim)?;
    let contributed: Vec<(&str, mush_economy::DeclaredCadence)> = Vec::new();
    // Doc 18 cut IC5 (D-18-33, §4.4): the built-ins of a roster that composes the
    // TacSim layer — `mush-inventory` names `tacsim` its third quantity delegate,
    // and the inventory handle cell comes back for the layer below.
    let weather = config_ext::weather_config(&config.weather);
    let economy = config_ext::economy_config_with(&config.economy, &contributed)?;
    // The ordinary operator recovery policy reaches both roster choices.
    let recovery = modules::recovery_policy(&tacsim.recovery);
    let (mut mods, inventory) = match (launch.as_ref(), commerce) {
        (Some(launch), None) => modules::server_modules_with_launch(
            weather,
            economy,
            atlas,
            &onboarding,
            &game,
            recovery,
            launch,
        )
        .map_err(|e| e.to_string())?,
        (_, Some(commerce)) => modules::server_modules_with_tacsim_and_mind_commerce(
            weather,
            economy,
            atlas,
            &onboarding,
            &game,
            recovery,
            commerce,
        )
        .map_err(|e| e.to_string())?,
        (None, None) => modules::server_modules_with_tacsim(
            weather,
            economy,
            atlas,
            &onboarding,
            &game,
            recovery,
        ),
    };
    mods.push(modules::worldgen_module_with(
        Arc::clone(llm),
        config_ext::worldgen_config(&config.worldgen),
    ));
    // M8: the NPC agent module, grounded by the game's sheet and on-loop abilities
    // through the generic ruleset grounding adapter.
    mods.push(modules::npc_module_with(
        modules::npc_agent_adapter(),
        &config.npc,
    ));
    // 16b cut R-b: the one game's character sheet and its rules interface.
    mods.extend(modules::sheet_modules(&onboarding, &game));
    // The TacSim layer, after the pilot sheet. Its config was
    // read above, before the module list, for `[tacsim.recovery]`'s sake.
    let tacsim_enabled = tacsim.enabled;
    // Say which way the layer went. Config::load catches near-miss headers;
    // a missing or deliberately disabled block still needs an operator warning.
    if tacsim_enabled {
        tracing::info!(
            packs = tacsim.packs.len(),
            seeds = tacsim.seeds.len(),
            "tacsim enabled: the simulator's arena and its stock tanks apply at boot"
        );
    } else {
        tracing::warn!(
            "tacsim disabled ([tacsim].enabled is false or the section is missing): no arena and \
             no stock tanks are applied, so the simulator centre cannot start a match"
        );
    }
    // Doc 18 cut IC2 (lead ruling R2): the same switch loads the game's own seed
    // packs (the lane) into the store, so `TacsimStore`'s loaded packs and the
    // content latch's boot list below move together.
    // Doc 18 cut IC5: handed the inventory cell (`tacsim_module_wired`, which takes
    // no test-world flag: production is never a test world, IC3 L15).
    mods.push(match (launch.as_ref(), commerce) {
        (Some(launch), None) => Box::new(
            launch
                .configure_tacsim(
                    tacsim::TacsimRuleset::with_game_content(tacsim, tacsim_enabled)
                        .with_conduct_carrier()
                        .map_err(|e| e.to_string())?
                        .with_inventory(inventory)
                        .with_unit_authority(),
                )
                .map_err(|e| e.to_string())?,
        ) as Box<dyn GameModule>,
        (_, Some(commerce)) => modules::tacsim_module_wired_with_mind_commerce(
            tacsim,
            tacsim_enabled,
            inventory,
            commerce,
        )
        .map_err(|error| error.to_string())?,
        (None, None) => modules::tacsim_module_wired(tacsim, tacsim_enabled, inventory),
    });
    // S1: the content module LAST of all — its boot pass applies the starter packs
    // through the atlas's and the NPC module's appliers, which must have hydrated
    // their own slots first (`on_world_loaded` runs in list order). Zero LLM calls:
    // the packs are bytes compiled into the server.
    if config.game.starter_area {
        tracing::info!("starter area on: the lot applies at a first boot");
    }
    // 16b cut C.2 (C3): with the layer on, the game's own two boot packs ride
    // beside the lot's — the proving ground's ground, then the arena whose
    // applier makes its muster, its domain and the twelve stock tanks.
    mods.push(modules::content_module_with(
        config.game.starter_area,
        tacsim_enabled,
    )?);
    Ok(mods)
}

/// What [`build_engine`] hands the serving half of the process.
pub struct BootedEngine {
    /// The composed, module-booted engine, not yet handed to the world task.
    pub engine: Engine,
    /// The staff queue the server commands push into and the world loop drains.
    /// It must be the SAME queue (`ServerDeps::staff`), or `@boot`/`@siteban`
    /// would enqueue onto an orphan.
    pub staff: Arc<StaffQueue>,
    /// The shared LLM service handle the modules were composed with.
    pub llm: Arc<LlmService>,
    /// `[llm].decision_workers`, read once here so the serving half never
    /// re-parses the stanza to find it.
    pub decision_workers: usize,
    /// Every composed module's namespace — the list the J5 nightly pass walks,
    /// which only the booted engine knows.
    pub archive_namespaces: Vec<String>,
}

/// Boot the full engine: the three durable key rings, the K3 recovery barrier,
/// the writable bootstrap, migrations, the accounts import, the module roster
/// ([`game_modules`]), the world load, and every module's boot pass.
///
/// The three key rings load BEFORE `bootstrap_writable`/`run_migrations` on
/// purpose: a boot that admitted a write before it could sign one would be a
/// world whose first offers, resolutions and decisions are unverifiable.
///
/// `validated_spool` is not read — it is the K0 preflight's evidence, borrowed
/// so this cannot be called on a boot path that never opened one.
///
/// # Errors
///
/// Unprovisionable key rings, a store that is not K3-durable, a failed recovery
/// barrier, failed migrations or accounts import, a rejected config stanza, a
/// module registration clash, or a module boot pass that fails.
pub fn build_engine<'a>(
    config: &'a Config,
    store: &'a Arc<dyn ObjectStore>,
    validated_spool: &'a QuarantineSpool,
) -> std::pin::Pin<Box<impl std::future::Future<Output = Result<BootedEngine, String>> + 'a>> {
    // Boot retains the composed engine and registry across store awaits. Keep
    // that state on the heap so each caller's async frame does not embed it
    // again. Nested boot/logging futures otherwise exhaust a normal thread's
    // stack in debug builds. Construction remains lazy: no boot work runs until
    // the returned future is polled, and the validated spool stays borrowed.
    Box::pin(build_engine_inner(config, store, validated_spool))
}

async fn build_engine_inner(
    config: &Config,
    store: &Arc<dyn ObjectStore>,
    _validated_spool: &QuarantineSpool,
) -> Result<BootedEngine, String> {
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
    // Doc 20 cut SM.1 (§3.2, Q-20-12, §16 "a lost key is a disabled layer, not a corrupted
    // one"): the geology key family — loaded only when the layer is on, and NEVER with `?`: a key
    // that will not load disables the mining layer, it does not refuse the boot. The three
    // protocol rings above keep refusing it. The key reaches the game crate after composition,
    // through `Engine::install_geology` (delegating to the registered `GeologyInstaller`), before
    // the module boot pass pins or compares its fingerprint.
    let geology = config
        .tacsim
        .enabled
        .then(|| match config.geology_key_dir() {
            None => mush_core::GeologyBoot::NotConfigured,
            Some(dir) => match DurableGeologyKeyStore::new(&dir).load_or_create() {
                Ok(secret) => {
                    tracing::info!(
                        version = secret.version(),
                        key_dir = %dir.display(),
                        "durable geology key loaded"
                    );
                    mush_core::GeologyBoot::Key(secret)
                }
                Err(error) => {
                    tracing::error!(
                        %error,
                        key_dir = %dir.display(),
                        "geology key did not load; the mining layer is disabled"
                    );
                    mush_core::GeologyBoot::Unreadable(error.to_string())
                }
            },
        });
    let authoritative = store.authoritative_store().ok_or_else(|| {
        "configured store does not implement K3 exact-token durability; refusing writable boot"
            .to_owned()
    })?;
    let recovery = reconcile_authoritative_for_writable_boot(authoritative).await?;
    tracing::info!(
        indexed = recovery.indexed,
        retained = recovery.retained,
        retained_bytes = recovery.retained_bytes,
        retention_counters_repaired = recovery.retention_counters_repaired,
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
    let version = run_migrations(&**store, &core_migrations())
        .await
        .map_err(|e| format!("store migrations failed: {e}"))?;
    tracing::info!(schema = version, "store schema current");
    let imported = import_accounts_json(&**store, &config.server.data_dir).await?;
    if imported > 0 {
        tracing::info!(imported, "M1 accounts.json imported");
    }
    // E7c5 B1: build the account-distinctness oracle from the store, once,
    // before composition (the store is in scope and async here). A legacy
    // account with no stored id is minted on the fly (deterministic); a
    // character claimed by two accounts is left unmapped and paged.
    let account_identity = crate::accounts_identity::hydrate(&**store).await?;
    tracing::info!(
        mapped_characters = account_identity.len(),
        "M-E7c5 account oracle hydrated"
    );
    let staff = Arc::new(StaffQueue::new());
    let mut registry = world_task::base_registry();
    register_server_commands_with_cap(&mut registry, &staff, config.game.max_characters as usize)
        .map_err(|e| format!("server command registration: {e}"))?;
    match crate::help::load_help_topics(&mut registry, &config.server.help_dir) {
        Ok(n) => tracing::info!(topics = n, "help topics loaded"),
        Err(e) => tracing::warn!(error = %e, dir = %config.server.help_dir.display(),
            "help topics failed to load; continuing without them"),
    }
    let llm_config = config_ext::llm_config(&config.llm)?;
    let llm =
        Arc::new(LlmService::from_config(&llm_config).map_err(|e| format!("llm config: {e}"))?);
    tracing::info!(
        enabled = llm_config.enabled,
        providers = llm_config.providers.len(),
        routes = llm_config.routes.len(),
        "llm service built"
    );
    // The roster, and every stanza it reads, in the one place that states
    // the composition order.
    let mods = game_modules(config, &llm)?;
    // Compose and freeze every mechanics registry before any persisted world
    // record is loaded. Rules-bearing attrs must never be interpreted against
    // a half-built definition/bridge set (K1 / C1 boot ordering).
    let composition =
        Engine::compose_modules(registry, mods).map_err(|e| format!("module registration: {e}"))?;
    let mut world = load_or_seed_world(&**store).await?;
    let clock = config.game.game_clock().map_err(|e| e.to_string())?;
    timeline::hydrate_world_timeline(&mut world, &**store, clock).await?;
    // WS04-S3: the all-in-one's own transcription of this root logged the
    // hydrated epoch and this one did not, so the split half installed a
    // persisted IC generation in silence. The deletion of that transcription
    // keeps the line rather than losing it.
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
    let account_identity: std::sync::Arc<dyn mush_core::AccountIdentity> =
        std::sync::Arc::new(account_identity);
    // Programme D4: the relay's `(account, actor)` re-validation reads the
    // same oracle from the engine.
    engine.set_account_identity(std::sync::Arc::clone(&account_identity));
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
    // S7 (doc 14 §5.4): the ground driver's cell size, on the same device. The
    // host validated it against the atlas's cell sizes (`config_ext`).
    if let Err(error) =
        mush_tactics::ground::configure(&mut engine.world, config.tacsim.ground_cell_cm)
    {
        tracing::warn!(%error, "boot: the ground driver's cell size did not apply");
    }
    // Doc 20 cut SM.1 (Q-20-12): the geology key, on the same post-composition device —
    // installed via `Engine::install_geology` before the boot pass that pins or compares the
    // key's fingerprint. With `[tacsim].enabled = false` nothing was loaded and the boot pass
    // reports the layer off from the store's own flag.
    if let Some(boot) = geology
        && !engine.install_geology(boot)
    {
        tracing::warn!("boot: no geology installer took the geology key");
    }
    modules::boot_modules(&mut engine, &**store).await?;
    // **16b cut F.4**: the simulator's readiness, once — and here, because this
    // is the first moment both halves of it are true or not. The centre's rooms
    // arrive with the starter area's content latch and the tanks arrive with the
    // game's boot packs, and both of those land inside the module boot pass
    // above. An operator who reads one line at boot never has to discover a
    // half-applied centre from a player's `SIM START`.
    //
    // **16b cut Z.3** counts the pods as well as the tanks, both against the
    // twelve this binary can seat. A standing world takes the Z.3 packs by hand,
    // and until it does it has eight of each and the lobby fields four a side
    // (`tacsim::match_::fieldable_per_side`): that world is **partly** ready, a
    // warning and not an error, because the centre works on what it has, and the
    // line names the route rather than calling it whole.
    if config.tacsim.enabled {
        let ready = tacsim::match_::centre_readiness(&engine.world);
        let whole = tacsim::match_::CentreReadiness::WHOLE;
        // The choice and the words are `CentreReadiness::warning`'s, pinned
        // there; this block only picks the level and the fields.
        if let Some(warning) = ready.warning() {
            if ready.rooms {
                tracing::warn!("{warning}");
            } else {
                tracing::warn!(roster_tanks = ready.tanks, "{warning}");
            }
        } else {
            let roster_tanks = format!("{} of {whole}", ready.tanks);
            let pods = format!("{} of {whole}", ready.pods);
            tracing::info!(
                %roster_tanks,
                %pods,
                "simulator centre ready: the arena is applied and its tanks are standing"
            );
            let configured = config.tacsim.match_.max_slots_per_side;
            let available = u32::from(tacsim::match_::PODS_PER_SIDE);
            if configured > available {
                tracing::warn!(
                    configured_slots_per_side = configured,
                    available_pods_per_side = available,
                    "simulator slots clamped to the centre's available pods"
                );
            }
        }
    }
    // J5: the namespaces the nightly archive pass walks — every composed
    // module's. Only the booted engine knows the list, which is why the pass
    // is scheduled by the caller and not inside `open_store`.
    let archive_namespaces: Vec<String> = engine
        .modules()
        .iter()
        .map(|m| m.name().to_string())
        .collect();
    Ok(BootedEngine {
        engine,
        staff,
        llm,
        decision_workers: llm_config.decision_workers,
        archive_namespaces,
    })
}

/// The world half's `--check-config` preflight: the config refusals
/// [`build_engine`] and [`spawn_ledger_archive`] raise from the parsed config
/// alone, before either touches the store, a socket or the quarantine lifetime
/// lock (WS04-S3).
///
/// Named rather than implied, because the earlier wording ("every config
/// refusal") was a promise this function did not keep — `[game].ic_epoch` was
/// boot-fatal here and unchecked there (W1-B8-FIX):
/// [`mush_config::preflight::check`] covers `[store].store`,
/// `[store].receipt_policy` and `[game].ic_epoch`, which all three binaries
/// read; the two added here need crates the gateway half does not depend on:
///
/// - `[tacsim]`, whose pack block is read and digest-verified host-side, so an
///   unreadable or mismatched pack is found by an operator rather than by a
///   supervisor watching a boot loop;
/// - `[store.archive].window`, which refuses the boot inside
///   [`spawn_ledger_archive`] rather than leaving the nightly pass silently
///   unscheduled for the life of the deployment.
///
/// `[ipc].listen` is deliberately absent: it is boot-fatal in the two split
/// halves and meaningless in the all-in-one, which reaches this function too,
/// so it hangs off [`mush_config::preflight::check_ipc`] and is added by the
/// split `main`s (W1-B7-FIX).
///
/// Refusals are collected, not short-circuited: a config with three mistakes
/// reports three.
#[must_use]
pub fn check_config(config: &Config) -> Vec<String> {
    let mut refusals = mush_config::preflight::check(config);
    if let Err(error) = config_ext::tacsim_config(&config.tacsim) {
        refusals.push(error);
    }
    if let Err(error) = load_launch_deployment(config) {
        refusals.push(error);
    }
    // S1: `[atlas].pack_paths` are read host-side and are boot-fatal for the same
    // reason `[tacsim].pack_paths` are — an unreadable or mis-digested pack refuses
    // the boot naming the file — so the preflight finds them at no cost too. The
    // call also validates `[worldgen].hydration_strikes_to_zero`, which reaches the
    // atlas through the same conversion.
    if let Err(error) = config_ext::atlas_config(&config.atlas, &config.worldgen) {
        refusals.push(error);
    }
    if config.store.archive.enabled
        && let Err(error) = ArchiveConfig::parse_window(&config.store.archive.window)
    {
        refusals.push(error);
    }
    refusals
}

/// Build the web `hello` manifest from the booted engine: every
/// module-registered panel feed plus the engine-side `"who"` and
/// `"legal.actions"`, and one feature id per module (comsys advertises as
/// `"channels"`). Returns `(features, panels)`.
#[must_use]
pub fn build_hello_manifest(engine: &Engine) -> (Vec<String>, Vec<String>) {
    let mut panels: Vec<String> = engine
        .registry
        .panel_feeds()
        .iter()
        .map(|(feed, _)| feed.to_string())
        .collect();
    panels.push("who".to_string());
    panels.push("legal.actions".to_string());
    let features: Vec<String> = engine
        .modules()
        .iter()
        .map(|m| match m.name() {
            "comsys" => "channels".to_string(),
            other => other.to_string(),
        })
        .collect();
    (features, panels)
}

//! The boot root both deployments share: open the store, schedule the J5
//! ledger archive pass, and describe the booted world to the gateway.
//!
//! Before WS04-S1 each half owned half of this. `mush-server` validated
//! `[store].receipt_policy` and never opened a [`LedgerArchive`]; `mushd`
//! opened the archive, attached it to both backends and scheduled the nightly
//! pass, and (until WS04-S0) never read the receipt policy at all. A knob
//! wired into one root only is a knob that silently does nothing on the other,
//! and the fork is meant to run the same world twice — so both live here, once,
//! and both roots call it.
//!
//! Three things this module owns:
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
//!   workstream exists to abolish.
//! - [`build_hello_manifest`] — the features/panels list the gateway turns
//!   into the web client's `hello` frame.

use std::sync::Arc;

use mush_config::config::{Config, StoreBackend, StoreReceiptPolicy};
use mush_config::metrics::Metrics;
use mush_core::Engine;
use mush_store::{InMemoryStore, LedgerArchive, ObjectStore, RedisStore};
use tokio::task::JoinHandle;

use crate::archive::{ArchiveConfig, spawn_archive_job};

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

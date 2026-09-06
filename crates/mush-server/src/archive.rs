//! **The ledger archive pass** (J5, doc 12 §10): inside a nightly maintenance
//! window, copy every sealed ledger chunk the primary store holds and the
//! archive does not to the compressed archive tier, read each back and verify
//! it, and — only then, only above a high watermark of ledger bytes resident
//! in the primary, and only for chunks older than the retention floor — purge
//! the primary of them down to a low watermark, oldest first.
//!
//! What the direction fixed and the code keeps:
//!
//! - **The unit is the sealed chunk.** The open tail is never copied and never
//!   purged; a chunk sealed during a pass is picked up the next night.
//! - **The purge gate is the verified copy.** Two nightly chances are not the
//!   safety mechanism — the read-back hash is; the 48 h floor keeps recent
//!   history hot for account views and gives two nights of grace on top. The
//!   floor is measured from when the archive copied the chunk (the ledger is
//!   payload-agnostic; it has no entry timestamp of its own — J5-D2).
//! - **Nothing purges on a failed pass.** Any copy or verify error ends the
//!   pass before the purge phase; a purge call that fails ends the purge.
//! - **Watermarks in pairs**, measured as ledger-family bytes resident in the
//!   primary (what a purge can reduce), never instance memory.
//! - **Reads fall through at the store** ([`mush_store::LedgerArchive`] attached
//!   to the backend), so a full-replay boot, H1's cold reads, scenes and canon
//!   read the archive without knowing it exists and the journal's authority
//!   spans both tiers — which is what keeps a checkpoint (J3) a projection.

use std::sync::Arc;
use std::time::Duration;

use mush_config::metrics::{LedgerArchiveSample, Metrics};
use mush_store::{LedgerArchive, ObjectStore};

/// The pass's knobs, from `[store.archive]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveConfig {
    /// The window's start, minutes after UTC midnight.
    pub window_start_min: u32,
    /// The window's end (exclusive), minutes after UTC midnight; may be below
    /// the start for a window that crosses midnight.
    pub window_end_min: u32,
    /// Never purge a chunk the archive copied less than this many hours ago.
    pub retain_hours: u64,
    /// Ledger bytes resident in the primary that start a purge.
    pub high_watermark_bytes: u64,
    /// … and where the purge stops.
    pub low_watermark_bytes: u64,
    /// Chunks per segment file.
    pub segment_chunks: usize,
    /// The most chunks one pass purges (bounded per night).
    pub max_purge_per_pass: usize,
    /// The module namespaces the pass walks (every composed module's).
    pub namespaces: Vec<String>,
}

impl ArchiveConfig {
    /// Parse a `HH:MM-HH:MM` UTC window into minutes after midnight.
    ///
    /// # Errors
    /// Not two `HH:MM` times joined by `-`, or a time out of range.
    pub fn parse_window(window: &str) -> Result<(u32, u32), String> {
        let (a, b) = window
            .split_once('-')
            .ok_or_else(|| format!("archive window {window:?} is not HH:MM-HH:MM"))?;
        Ok((parse_hhmm(a.trim())?, parse_hhmm(b.trim())?))
    }
}

fn parse_hhmm(s: &str) -> Result<u32, String> {
    let (h, m) = s
        .split_once(':')
        .ok_or_else(|| format!("archive window time {s:?} is not HH:MM"))?;
    let h: u32 = h
        .parse()
        .map_err(|_| format!("archive window hour {h:?} is not a number"))?;
    let m: u32 = m
        .parse()
        .map_err(|_| format!("archive window minute {m:?} is not a number"))?;
    if h > 23 || m > 59 {
        return Err(format!("archive window time {s:?} is out of range"));
    }
    Ok(h * 60 + m)
}

/// Whether `now` (unix seconds) falls inside the UTC window.
#[must_use]
pub fn in_window(cfg: &ArchiveConfig, now: i64) -> bool {
    let minute = u32::try_from(now.rem_euclid(86_400) / 60).unwrap_or(0);
    let (start, end) = (cfg.window_start_min, cfg.window_end_min);
    if start <= end {
        minute >= start && minute < end
    } else {
        minute >= start || minute < end
    }
}

/// The UTC day `now` falls in (one pass a day).
#[must_use]
pub fn utc_day(now: i64) -> i64 {
    now.div_euclid(86_400)
}

/// What one pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PassReport {
    /// Sealed chunks copied to the archive and read back this pass.
    pub copied: usize,
    /// Open tails seen and left alone.
    pub skipped_open: usize,
    /// Chunks purged from the primary this pass.
    pub purged: usize,
    /// Ledger bytes resident in the primary after the pass.
    pub resident_bytes: u64,
    /// Chunks the archive holds after the pass.
    pub archive_chunks: u64,
    /// Whether the purge phase ran (the high watermark was crossed).
    pub purge_considered: bool,
    /// Non-fatal problems (a purge call that failed).
    pub warnings: Vec<String>,
}

/// One chunk resident in the primary, as the pass saw it.
struct Resident {
    ns: String,
    log_id: String,
    chunk: u64,
    key: String,
    raw_len: u64,
}

/// Run one pass now. Copy and verify first; then purge only if the primary is
/// above the high watermark.
///
/// # Errors
/// A store listing or read failed, or the archive refused a copy: the pass
/// ends **before** any purge.
pub async fn run_pass(
    store: &dyn ObjectStore,
    archive: &LedgerArchive,
    cfg: &ArchiveConfig,
    now: i64,
) -> Result<PassReport, String> {
    let mut report = PassReport::default();
    let mut resident: Vec<Resident> = Vec::new();
    // ---- phase 1: copy and verify every sealed chunk the archive lacks ----
    for ns in &cfg.namespaces {
        let keys = store
            .module_keys(ns)
            .await
            .map_err(|e| format!("listing {ns:?}: {e}"))?;
        let mut staged: std::collections::BTreeMap<String, Vec<(u64, String)>> =
            std::collections::BTreeMap::new();
        for key in keys {
            let Some((log_id, chunk)) = mush_core::parse_chunk_key(&key) else {
                continue;
            };
            let log_id = log_id.to_string();
            if let Some(loc) = archive.holds(ns, &log_id, chunk) {
                // Held and still resident: only its size matters to the purge.
                resident.push(Resident {
                    ns: ns.clone(),
                    log_id,
                    chunk,
                    key,
                    raw_len: loc.raw_len,
                });
                continue;
            }
            let Some(text) = store
                .module_doc_text(ns, &key)
                .await
                .map_err(|e| format!("reading {ns}:{key}: {e}"))?
            else {
                continue; // gone between the listing and the read
            };
            let raw_len = u64::try_from(text.len()).unwrap_or(u64::MAX);
            match mush_core::parse_chunk(&text) {
                Ok(parsed)
                    if parsed.sealed && parsed.chunk == chunk && !parsed.entries.is_empty() =>
                {
                    let contiguous = parsed
                        .entries
                        .windows(2)
                        .all(|pair| pair[0].seq.checked_add(1) == Some(pair[1].seq));
                    if contiguous {
                        staged
                            .entry(log_id.clone())
                            .or_default()
                            .push((chunk, text));
                    } else {
                        tracing::warn!(
                            ns,
                            log_id,
                            chunk,
                            "ledger archive: a sealed chunk with a sequence gap is left in place"
                        );
                    }
                }
                Ok(_) => {
                    report.skipped_open += 1;
                }
                Err(error) => {
                    tracing::warn!(ns, %key, %error, "ledger archive: a log: key that is not a chunk is left in place");
                }
            }
            resident.push(Resident {
                ns: ns.clone(),
                log_id,
                chunk,
                key,
                raw_len,
            });
        }
        for (log_id, chunks) in staged {
            let admitted = archive
                .store_chunks(ns, &log_id, chunks, now, cfg.segment_chunks)
                .map_err(|e| format!("archiving {ns}/{log_id}: {e}"))?;
            report.copied += admitted.len();
        }
    }
    report.resident_bytes = resident.iter().map(|r| r.raw_len).sum();
    report.archive_chunks = archive.stats().chunks;

    // ---- phase 2: purge, only above the high watermark ----
    if report.resident_bytes <= cfg.high_watermark_bytes {
        return Ok(report);
    }
    report.purge_considered = true;
    let floor = now
        .saturating_sub(i64::try_from(cfg.retain_hours.saturating_mul(3_600)).unwrap_or(i64::MAX));
    // Eligible: resident, held by the archive (verified at copy), unpurged,
    // and archived at or before the floor. Oldest copies first.
    let mut eligible: Vec<(i64, Resident)> = resident
        .into_iter()
        .filter_map(|r| {
            let loc = archive.holds(&r.ns, &r.log_id, r.chunk)?;
            (!loc.purged && loc.archived_at <= floor).then_some((loc.archived_at, r))
        })
        .collect();
    eligible.sort_by(|a, b| {
        (a.0, &a.1.ns, &a.1.log_id, a.1.chunk).cmp(&(b.0, &b.1.ns, &b.1.log_id, b.1.chunk))
    });
    let mut resident_bytes = report.resident_bytes;
    let mut batch: Vec<Resident> = Vec::new();
    let mut purged = 0usize;
    for (_, r) in eligible {
        if resident_bytes <= cfg.low_watermark_bytes
            || purged + batch.len() >= cfg.max_purge_per_pass
        {
            break;
        }
        resident_bytes = resident_bytes.saturating_sub(r.raw_len);
        if let Some(head) = batch.first()
            && (head.ns != r.ns || head.log_id != r.log_id)
        {
            purged += purge_batch(store, archive, &mut report, std::mem::take(&mut batch)).await;
        }
        batch.push(r);
        if !report.warnings.is_empty() {
            break;
        }
    }
    if !batch.is_empty() && report.warnings.is_empty() {
        purged += purge_batch(store, archive, &mut report, batch).await;
    }
    report.purged = purged;
    report.resident_bytes = resident_bytes.max(
        report
            .resident_bytes
            .saturating_sub(report.resident_bytes.saturating_sub(resident_bytes)),
    );
    Ok(report)
}

/// Purge one `(ns, log)` batch from the primary and mark it in the archive.
/// A failed purge call is a warning on the report and ends purging.
async fn purge_batch(
    store: &dyn ObjectStore,
    archive: &LedgerArchive,
    report: &mut PassReport,
    batch: Vec<Resident>,
) -> usize {
    let Some(head) = batch.first() else {
        return 0;
    };
    let (ns, log_id) = (head.ns.clone(), head.log_id.clone());
    let keys: Vec<String> = batch.iter().map(|r| r.key.clone()).collect();
    let chunks: Vec<u64> = batch.iter().map(|r| r.chunk).collect();
    match store.module_purge_archived(&ns, keys).await {
        Ok(removed) => {
            if let Err(error) = archive.mark_purged(&ns, &log_id, &chunks) {
                report
                    .warnings
                    .push(format!("marking {ns}/{log_id} purged: {error}"));
            }
            removed
        }
        Err(error) => {
            report
                .warnings
                .push(format!("purging {ns}/{log_id}: {error}"));
            0
        }
    }
}

/// The nightly job: once a UTC day, inside the window, run a pass and publish
/// what it did. Runs beside the store writer, never on the world task.
pub fn spawn_archive_job(
    store: Arc<dyn ObjectStore>,
    archive: Arc<LedgerArchive>,
    cfg: ArchiveConfig,
    metrics: Arc<Metrics>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut last_day: Option<i64> = None;
        let stats = archive.stats();
        metrics.publish_ledger_archive(LedgerArchiveSample {
            chunks: i64::try_from(stats.chunks).unwrap_or(i64::MAX),
            purged_chunks: i64::try_from(stats.purged).unwrap_or(i64::MAX),
            resident_bytes: -1,
            last_pass_unix: 0,
            last_pass_ok: true,
        });
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let now = unix_now();
            if !in_window(&cfg, now) || last_day == Some(utc_day(now)) {
                continue;
            }
            last_day = Some(utc_day(now));
            match run_pass(&*store, &archive, &cfg, now).await {
                Ok(report) => {
                    tracing::info!(
                        copied = report.copied,
                        purged = report.purged,
                        skipped_open = report.skipped_open,
                        resident_bytes = report.resident_bytes,
                        archive_chunks = report.archive_chunks,
                        warnings = report.warnings.len(),
                        "ledger archive: pass complete"
                    );
                    for warning in &report.warnings {
                        tracing::warn!(%warning, "ledger archive: pass warning");
                    }
                    let stats = archive.stats();
                    metrics.publish_ledger_archive(LedgerArchiveSample {
                        chunks: i64::try_from(stats.chunks).unwrap_or(i64::MAX),
                        purged_chunks: i64::try_from(stats.purged).unwrap_or(i64::MAX),
                        resident_bytes: i64::try_from(report.resident_bytes).unwrap_or(i64::MAX),
                        last_pass_unix: now,
                        last_pass_ok: report.warnings.is_empty(),
                    });
                }
                Err(error) => {
                    tracing::error!(%error, "ledger archive: pass failed; nothing purged");
                    metrics.publish_ledger_archive(LedgerArchiveSample {
                        chunks: i64::try_from(archive.stats().chunks).unwrap_or(i64::MAX),
                        purged_chunks: i64::try_from(archive.stats().purged).unwrap_or(i64::MAX),
                        resident_bytes: -1,
                        last_pass_unix: now,
                        last_pass_ok: false,
                    });
                }
            }
        }
    })
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{ModuleOp, ModuleWrite, chunk_key, inspect_ledger};
    use mush_store::{InMemoryStore, WriteBatch};
    use std::path::PathBuf;

    const NS: &str = "economy";
    const LOG: &str = "journal";
    const HOUR: i64 = 3_600;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rustmush_archive_pass_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn chunk_doc(chunk: u64, first_seq: u64, n: u64, sealed: bool) -> serde_json::Value {
        let entries: Vec<serde_json::Value> = (first_seq..first_seq + n)
            .map(|seq| serde_json::json!({ "seq": seq, "value": { "n": seq } }))
            .collect();
        serde_json::json!({ "chunk": chunk, "sealed": sealed, "entries": entries })
    }

    /// A store holding `sealed` sealed chunks, an open tail after them, and one
    /// ordinary document.
    async fn seeded(sealed: u64, archive: Option<Arc<LedgerArchive>>) -> Arc<InMemoryStore> {
        let store = match archive {
            Some(a) => InMemoryStore::new().with_archive(a),
            None => InMemoryStore::new(),
        };
        let mut ops: Vec<ModuleOp> = Vec::new();
        for c in 0..sealed {
            ops.push(ModuleOp {
                ns: NS.into(),
                op: ModuleWrite::JsonSet {
                    key: chunk_key(LOG, c),
                    value: chunk_doc(c, c * 2, 2, true),
                },
            });
        }
        ops.push(ModuleOp {
            ns: NS.into(),
            op: ModuleWrite::JsonSet {
                key: chunk_key(LOG, sealed),
                value: chunk_doc(sealed, sealed * 2, 1, false),
            },
        });
        ops.push(ModuleOp {
            ns: NS.into(),
            op: ModuleWrite::JsonSet {
                key: "account:1".into(),
                value: serde_json::json!({ "balance": 5 }),
            },
        });
        store
            .write(WriteBatch {
                serial: 1,
                module_ops: ops,
                ..WriteBatch::default()
            })
            .await
            .unwrap();
        Arc::new(store)
    }

    fn cfg(high_bytes: u64, low_bytes: u64, retain_hours: u64) -> ArchiveConfig {
        ArchiveConfig {
            window_start_min: 0,
            window_end_min: 24 * 60,
            retain_hours,
            high_watermark_bytes: high_bytes,
            low_watermark_bytes: low_bytes,
            segment_chunks: 2,
            max_purge_per_pass: 4_096,
            namespaces: vec![NS.to_string(), "scenes".to_string()],
        }
    }

    /// Lens O: sealed chunks are copied and verified; the open tail and the
    /// ordinary document are left alone; a second pass copies nothing; nothing
    /// is purged below the high watermark.
    #[tokio::test]
    async fn a_pass_copies_sealed_chunks_and_leaves_the_open_tail() {
        let root = temp_root("copy");
        let archive = Arc::new(LedgerArchive::open(&root).unwrap());
        let store = seeded(3, Some(Arc::clone(&archive))).await;
        let report = run_pass(&*store, &archive, &cfg(u64::MAX, 0, 48), 1_000)
            .await
            .unwrap();
        assert_eq!(
            (report.copied, report.skipped_open, report.purged),
            (3, 1, 0)
        );
        assert!(!report.purge_considered);
        assert_eq!(report.archive_chunks, 3);
        assert!(report.warnings.is_empty());
        for c in 0..3 {
            assert!(archive.holds(NS, LOG, c).is_some(), "chunk {c} archived");
        }
        assert!(
            archive.holds(NS, LOG, 3).is_none(),
            "the open tail is never copied"
        );
        assert_eq!(
            store.module_keys(NS).await.unwrap().len(),
            5,
            "nothing purged"
        );
        let again = run_pass(&*store, &archive, &cfg(u64::MAX, 0, 48), 2_000)
            .await
            .unwrap();
        assert_eq!(again.copied, 0, "idempotent");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Lenses B (the floor), H (the boot still sees everything), S (the tier):
    /// above the high watermark the pass purges only chunks archived at least
    /// `retain_hours` ago, oldest first, down to the low watermark; the purged
    /// chunks still answer through the store, and a boot-shaped listing over
    /// the merged tiers is contiguous.
    #[tokio::test]
    async fn purge_waits_for_the_floor_then_the_tier_serves_what_left() {
        let root = temp_root("purge");
        let archive = Arc::new(LedgerArchive::open(&root).unwrap());
        let store = seeded(4, Some(Arc::clone(&archive))).await;
        let t0 = 10_000;
        // Every byte is over the (zero) high watermark, but nothing is 48 h old.
        let first = run_pass(&*store, &archive, &cfg(0, 0, 48), t0)
            .await
            .unwrap();
        assert_eq!((first.copied, first.purged), (4, 0));
        assert!(first.purge_considered);
        assert_eq!(store.module_keys(NS).await.unwrap().len(), 6);
        // 49 h later: purge down to the low watermark, oldest first.
        let later = t0 + 49 * HOUR;
        let second = run_pass(&*store, &archive, &cfg(0, 0, 48), later)
            .await
            .unwrap();
        assert_eq!(second.copied, 0);
        assert_eq!(
            second.purged, 4,
            "every archived chunk was eligible: {second:?}"
        );
        let keys = store.module_keys(NS).await.unwrap();
        assert!(!keys.iter().any(|k| k == "log:journal:0"), "{keys:?}");
        assert!(
            keys.iter().any(|k| k == "log:journal:4"),
            "the open tail stays: {keys:?}"
        );
        assert!(keys.iter().any(|k| k == "account:1"));
        assert_eq!(archive.stats().purged, 4);
        // The tier: a purged chunk still answers by key and by text.
        let doc = store
            .module_json_get(NS, "log:journal:1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc["chunk"], 1);
        assert!(
            store
                .module_doc_text(NS, "log:journal:0")
                .await
                .unwrap()
                .is_some()
        );
        // A boot-shaped listing merges both tiers and is contiguous.
        let docs = store.module_docs(NS).await.unwrap();
        assert_eq!(
            docs.ledger.len(),
            5,
            "{:?}",
            docs.ledger.iter().map(|(k, _)| k).collect::<Vec<_>>()
        );
        let ledger: Vec<mush_core::LedgerDoc> = docs
            .ledger
            .into_iter()
            .map(|(k, t)| (smol_str::SmolStr::new(k), t))
            .collect();
        let history = inspect_ledger(&ledger, None);
        assert!(history.is_clean(), "{:?}", history.breaches());
        assert_eq!(history.entries(), 9);
        // The resident bytes the report counts are the open tail plus the doc it ignores.
        assert!(second.resident_bytes < first.resident_bytes);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Lens V: a chunk present in both tiers is served from the primary even
    /// when the archive's copy differs; a chunk damaged in the archive after
    /// the purge is absent, and the merged listing is then a contiguity breach
    /// — a fail-closed boot, never a silent gap.
    #[tokio::test]
    async fn the_primary_wins_and_damage_is_a_breach() {
        let root = temp_root("damage");
        let archive = Arc::new(LedgerArchive::open(&root).unwrap());
        // The archive holds a chunk 0 whose bytes differ from the primary's.
        archive
            .store_chunks(
                NS,
                LOG,
                vec![(
                    0,
                    serde_json::to_string(&chunk_doc(0, 0, 2, true))
                        .unwrap()
                        .replace("\"n\":1", "\"n\":9"),
                )],
                1,
                256,
            )
            .unwrap();
        let store = seeded(2, Some(Arc::clone(&archive))).await;
        let doc = store
            .module_json_get(NS, "log:journal:0")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(doc["entries"][1]["value"]["n"], 1, "the primary is served");
        // With no retention floor a chunk is eligible the pass it is copied in:
        // copy, verify and purge in one pass. Then damage the copy of chunk 1.
        let purged = run_pass(&*store, &archive, &cfg(0, 0, 0), 1_000)
            .await
            .unwrap();
        assert_eq!((purged.copied, purged.purged), (1, 2), "{purged:?}");
        let loc = archive.holds(NS, LOG, 1).unwrap();
        let seg = root.join(NS).join(LOG).join(&loc.segment);
        let mut bytes = std::fs::read(&seg).unwrap();
        let at = usize::try_from(loc.offset + loc.len / 2).unwrap();
        bytes[at] ^= 0xff;
        std::fs::write(&seg, bytes).unwrap();
        assert!(
            store
                .module_json_get(NS, "log:journal:1")
                .await
                .unwrap()
                .is_none()
        );
        let docs = store.module_docs(NS).await.unwrap();
        let ledger: Vec<mush_core::LedgerDoc> = docs
            .ledger
            .into_iter()
            .map(|(k, t)| (smol_str::SmolStr::new(k), t))
            .collect();
        let history = inspect_ledger(&ledger, None);
        assert!(!history.is_clean(), "damage is a breach at boot");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Lens T: a crash between the segment and its index leaves an orphan the
    /// next open sweeps, and the next pass copies the chunk again.
    #[tokio::test]
    async fn an_orphan_segment_is_swept_and_the_chunk_copied_again() {
        let root = temp_root("orphan");
        let archive = Arc::new(LedgerArchive::open(&root).unwrap());
        let store = seeded(2, Some(Arc::clone(&archive))).await;
        let _ = run_pass(&*store, &archive, &cfg(u64::MAX, 0, 48), 1_000)
            .await
            .unwrap();
        let dir = root.join(NS).join(LOG);
        // Drop the index of the one segment: what a crash after the segment
        // write and before the index write leaves behind.
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.to_string_lossy().ends_with(".idx.json") {
                std::fs::remove_file(path).unwrap();
            }
        }
        drop(archive);
        let reopened = Arc::new(LedgerArchive::open(&root).unwrap());
        assert!(reopened.holds(NS, LOG, 0).is_none(), "the orphan was swept");
        let store = seeded(2, Some(Arc::clone(&reopened))).await;
        let again = run_pass(&*store, &reopened, &cfg(u64::MAX, 0, 48), 2_000)
            .await
            .unwrap();
        assert_eq!(again.copied, 2);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Lens W: the window and the day gate.
    #[test]
    fn the_window_is_utc_and_may_cross_midnight() {
        assert_eq!(
            ArchiveConfig::parse_window("03:00-04:00").unwrap(),
            (180, 240)
        );
        assert_eq!(
            ArchiveConfig::parse_window("23:30-00:30").unwrap(),
            (1_410, 30)
        );
        assert!(ArchiveConfig::parse_window("3-4").is_err());
        assert!(ArchiveConfig::parse_window("25:00-26:00").is_err());
        let mut c = cfg(0, 0, 0);
        c.window_start_min = 180;
        c.window_end_min = 240;
        assert!(in_window(&c, 3 * HOUR + 1));
        assert!(!in_window(&c, 4 * HOUR));
        assert!(!in_window(&c, 2 * HOUR));
        c.window_start_min = 1_410;
        c.window_end_min = 30;
        assert!(in_window(&c, 23 * HOUR + 45 * 60));
        assert!(in_window(&c, 10 * 60));
        assert!(!in_window(&c, 12 * HOUR));
        assert_eq!(utc_day(86_399), 0);
        assert_eq!(utc_day(86_400), 1);
    }

    /// The purge is bounded per pass and never reaches below the low watermark.
    #[tokio::test]
    async fn the_purge_is_bounded_and_stops_at_the_low_watermark() {
        let root = temp_root("bounded");
        let archive = Arc::new(LedgerArchive::open(&root).unwrap());
        let store = seeded(6, Some(Arc::clone(&archive))).await;
        let mut c = cfg(0, 0, 0);
        c.max_purge_per_pass = 2;
        let first = run_pass(&*store, &archive, &c, 1_000).await.unwrap();
        assert_eq!(
            (first.copied, first.purged),
            (6, 2),
            "bounded per pass: {first:?}"
        );
        let keys = store.module_keys(NS).await.unwrap();
        assert!(!keys.iter().any(|k| k == "log:journal:0"));
        assert!(!keys.iter().any(|k| k == "log:journal:1"));
        assert!(
            keys.iter().any(|k| k == "log:journal:2"),
            "oldest first: {keys:?}"
        );
        let second = run_pass(&*store, &archive, &c, 2_000).await.unwrap();
        assert_eq!(second.purged, 2, "{second:?}");
        let keys = store.module_keys(NS).await.unwrap();
        assert!(!keys.iter().any(|k| k == "log:journal:3"));
        assert!(keys.iter().any(|k| k == "log:journal:4"), "{keys:?}");
        // A low watermark above the resident total purges nothing more.
        let mut c2 = cfg(0, u64::MAX, 0);
        c2.max_purge_per_pass = 100;
        let report = run_pass(&*store, &archive, &c2, 3_000).await.unwrap();
        assert_eq!(report.purged, 0);
        let _ = std::fs::remove_dir_all(&root);
    }
}

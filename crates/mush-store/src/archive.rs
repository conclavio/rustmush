//! **The ledger archive tier** (doc 12 §10, J5): sealed ledger chunks copied
//! to compressed disk, so the primary store can be purged of history it no
//! longer needs resident while the journal's authority still spans both tiers.
//!
//! The unit is the **sealed chunk** — `log:{id}:{chunk}` with `sealed: true`,
//! rewritten on every append until it seals and never again — so a copy is
//! copy-once, verify-once and idempotent: a chunk the archive already holds is
//! hash-checked, not re-copied. The open tail is never eligible.
//!
//! ## On disk
//!
//! `<root>/<ns>/<log>/{first}-{last}.seg` holds a run of chunks, each its own
//! gzip member at a recorded offset, so one cold read inflates one chunk; the
//! sibling `{first}-{last}.idx.json` records every chunk's offset, length,
//! SHA-256 of the raw text, when it was archived, and whether the primary has
//! since been purged of it. A segment is written temp → fsync → rename, then
//! its index the same way, so a crash between them leaves a segment nobody
//! names — swept at the next [`LedgerArchive::open`] — and never a half-present
//! chunk. The index files are rebuildable side files: every segment is
//! self-describing through its own index.
//!
//! ## Trust
//!
//! The archive is read-only to the server; only the archive pass writes it.
//! A chunk whose bytes do not hash to what the index recorded is **absent**
//! ([`LedgerArchive::read_chunk`] returns `Ok(None)` and says why at `error`):
//! at boot that is a contiguity breach and a fail-closed boot, which is the
//! right failure for money history. Presence in both tiers is served from the
//! primary and compared; a disagreement is reported, never hidden.
//!
//! The archive is money and belongs in the same backup set as the primary.

use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::StoreError;

/// The most chunks one segment file holds by default (doc 12 §10.4).
pub const DEFAULT_SEGMENT_CHUNKS: usize = 256;

/// Where one archived chunk lives and what it hashed to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkLoc {
    /// The segment file name (`{first}-{last}.seg`), relative to the log's dir.
    pub segment: String,
    /// Byte offset of the chunk's gzip member in the segment.
    pub offset: u64,
    /// Length of the gzip member.
    pub len: u64,
    /// Length of the raw chunk text (what the primary held).
    pub raw_len: u64,
    /// SHA-256 of the raw chunk text, hex.
    pub sha256: String,
    /// When the archive copied it (wall-clock seconds).
    pub archived_at: i64,
    /// Whether the archive pass has since purged it from the primary store.
    #[serde(default)]
    pub purged: bool,
}

/// One segment's index file.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SegmentIndex {
    ns: String,
    log_id: String,
    chunks: BTreeMap<u64, ChunkLoc>,
}

#[derive(Debug, Default)]
struct LogIndex {
    chunks: BTreeMap<u64, ChunkLoc>,
}

#[derive(Debug, Default)]
struct ArchiveIndex {
    logs: BTreeMap<(String, String), LogIndex>,
}

/// The archive: a root directory and the in-RAM index of every segment under
/// it.
#[derive(Debug)]
pub struct LedgerArchive {
    root: PathBuf,
    index: Mutex<ArchiveIndex>,
}

/// Totals over the archive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArchiveStats {
    /// Chunks the archive holds.
    pub chunks: u64,
    /// Of those, chunks the primary has been purged of.
    pub purged: u64,
    /// Raw bytes of every held chunk (what the primary held or holds).
    pub raw_bytes: u64,
    /// Compressed bytes on disk.
    pub compressed_bytes: u64,
}

fn archive_error(what: impl Into<String>) -> StoreError {
    StoreError::Archive(what.into())
}

/// A directory-safe spelling of a namespace or log id: alphanumerics, `-`
/// and `_` pass; everything else is `%XX`.
fn safe_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

fn segment_name(first: u64, last: u64) -> String {
    format!("{first:012}-{last:012}.seg")
}

fn index_name(segment: &str) -> String {
    format!("{}.idx.json", segment.trim_end_matches(".seg"))
}

/// Write `bytes` to `path` durably: temp file, fsync, rename over.
fn write_durably(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    {
        let mut file = fs::File::create(&tmp)
            .map_err(|e| archive_error(format!("create {}: {e}", tmp.display())))?;
        file.write_all(bytes)
            .map_err(|e| archive_error(format!("write {}: {e}", tmp.display())))?;
        file.sync_all()
            .map_err(|e| archive_error(format!("fsync {}: {e}", tmp.display())))?;
    }
    fs::rename(&tmp, path).map_err(|e| {
        archive_error(format!(
            "rename {} → {}: {e}",
            tmp.display(),
            path.display()
        ))
    })?;
    if let Some(dir) = path.parent()
        && let Ok(d) = fs::File::open(dir)
    {
        // Best effort: the directory entry itself.
        let _ = d.sync_all();
    }
    Ok(())
}

impl LedgerArchive {
    /// Open (creating) the archive at `root`: load every segment index under
    /// it, and sweep orphans — a `.seg` with no `.idx.json` (a crash between
    /// the two writes) and any leftover `.tmp`.
    ///
    /// # Errors
    /// The directory cannot be created or read, or an index file is unreadable.
    pub fn open(root: impl Into<PathBuf>) -> Result<LedgerArchive, StoreError> {
        let root = root.into();
        fs::create_dir_all(&root)
            .map_err(|e| archive_error(format!("create {}: {e}", root.display())))?;
        let mut index = ArchiveIndex::default();
        let mut orphans = 0usize;
        for ns_entry in read_dir(&root)? {
            if !ns_entry.is_dir() {
                continue;
            }
            for log_entry in read_dir(&ns_entry)? {
                if !log_entry.is_dir() {
                    continue;
                }
                let mut segments: Vec<String> = Vec::new();
                let mut indexes: Vec<String> = Vec::new();
                for file in read_dir(&log_entry)? {
                    let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
                        continue;
                    };
                    if name.ends_with(".tmp") {
                        let _ = fs::remove_file(&file);
                        orphans += 1;
                    } else if name.ends_with(".seg") {
                        segments.push(name.to_owned());
                    } else if name.ends_with(".idx.json") {
                        indexes.push(name.to_owned());
                    }
                }
                for segment in &segments {
                    if !indexes.contains(&index_name(segment)) {
                        let _ = fs::remove_file(log_entry.join(segment));
                        orphans += 1;
                        tracing::warn!(
                            segment = %log_entry.join(segment).display(),
                            "ledger archive: segment with no index swept (a crash between the two writes)"
                        );
                    }
                }
                for idx in &indexes {
                    let path = log_entry.join(idx);
                    let text = fs::read_to_string(&path)
                        .map_err(|e| archive_error(format!("read {}: {e}", path.display())))?;
                    let seg: SegmentIndex = serde_json::from_str(&text)
                        .map_err(|e| archive_error(format!("decode {}: {e}", path.display())))?;
                    let log = index
                        .logs
                        .entry((seg.ns.clone(), seg.log_id.clone()))
                        .or_default();
                    for (chunk, loc) in seg.chunks {
                        log.chunks.insert(chunk, loc);
                    }
                }
            }
        }
        if orphans > 0 {
            tracing::warn!(orphans, root = %root.display(), "ledger archive: orphans swept at open");
        }
        Ok(LedgerArchive {
            root,
            index: Mutex::new(index),
        })
    }

    /// The archive's root directory.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn log_dir(&self, ns: &str, log_id: &str) -> PathBuf {
        self.root
            .join(safe_component(ns))
            .join(safe_component(log_id))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ArchiveIndex> {
        self.index
            .lock()
            .expect("ledger archive index lock poisoned")
    }

    /// Where `chunk` of `log_id` in `ns` lives, if the archive holds it.
    #[must_use]
    pub fn holds(&self, ns: &str, log_id: &str, chunk: u64) -> Option<ChunkLoc> {
        self.lock()
            .logs
            .get(&(ns.to_owned(), log_id.to_owned()))
            .and_then(|log| log.chunks.get(&chunk).cloned())
    }

    /// Every chunk the archive holds for `log_id` in `ns`, in index order.
    #[must_use]
    pub fn chunks_of(&self, ns: &str, log_id: &str) -> Vec<(u64, ChunkLoc)> {
        self.lock()
            .logs
            .get(&(ns.to_owned(), log_id.to_owned()))
            .map(|log| {
                log.chunks
                    .iter()
                    .map(|(chunk, loc)| (*chunk, loc.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every log the archive holds anything for in `ns`.
    #[must_use]
    pub fn logs_of(&self, ns: &str) -> Vec<String> {
        self.lock()
            .logs
            .keys()
            .filter(|(n, _)| n == ns)
            .map(|(_, log)| log.clone())
            .collect()
    }

    /// Totals over the archive.
    #[must_use]
    pub fn stats(&self) -> ArchiveStats {
        let index = self.lock();
        let mut stats = ArchiveStats::default();
        for log in index.logs.values() {
            for loc in log.chunks.values() {
                stats.chunks += 1;
                if loc.purged {
                    stats.purged += 1;
                }
                stats.raw_bytes += loc.raw_len;
                stats.compressed_bytes += loc.len;
            }
        }
        stats
    }

    /// Copy `chunks` (`(chunk index, raw text)`) of `log_id` in `ns` into new
    /// segments of at most `segment_chunks` each, durably, then **read every
    /// one back** and compare its hash before the index admits it. Chunks the
    /// archive already holds are skipped. Returns the chunks admitted.
    ///
    /// The caller has established that every text is a **sealed** chunk
    /// (`mush_core::parse_chunk` + `sealed`); the archive stores bytes.
    ///
    /// # Errors
    /// Any I/O failure, or a read-back that does not hash to what was written
    /// — in which case nothing from that segment is admitted.
    pub fn store_chunks(
        &self,
        ns: &str,
        log_id: &str,
        chunks: Vec<(u64, String)>,
        now: i64,
        segment_chunks: usize,
    ) -> Result<Vec<u64>, StoreError> {
        let segment_chunks = segment_chunks.max(1);
        let mut fresh: BTreeMap<u64, String> = BTreeMap::new();
        {
            let index = self.lock();
            let held = index.logs.get(&(ns.to_owned(), log_id.to_owned()));
            for (chunk, text) in chunks {
                if held.is_some_and(|log| log.chunks.contains_key(&chunk)) {
                    continue;
                }
                fresh.insert(chunk, text);
            }
        }
        if fresh.is_empty() {
            return Ok(Vec::new());
        }
        let dir = self.log_dir(ns, log_id);
        fs::create_dir_all(&dir)
            .map_err(|e| archive_error(format!("create {}: {e}", dir.display())))?;
        let entries: Vec<(u64, String)> = fresh.into_iter().collect();
        let mut admitted = Vec::new();
        for run in entries.chunks(segment_chunks) {
            let first = run[0].0;
            let last = run[run.len() - 1].0;
            let segment = segment_name(first, last);
            let mut bytes: Vec<u8> = Vec::new();
            let mut locs: BTreeMap<u64, ChunkLoc> = BTreeMap::new();
            for (chunk, text) in run {
                let offset = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
                let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
                encoder
                    .write_all(text.as_bytes())
                    .map_err(|e| archive_error(format!("compress chunk {chunk}: {e}")))?;
                let member = encoder
                    .finish()
                    .map_err(|e| archive_error(format!("compress chunk {chunk}: {e}")))?;
                bytes.extend_from_slice(&member);
                locs.insert(
                    *chunk,
                    ChunkLoc {
                        segment: segment.clone(),
                        offset,
                        len: u64::try_from(member.len()).unwrap_or(u64::MAX),
                        raw_len: u64::try_from(text.len()).unwrap_or(u64::MAX),
                        sha256: sha256_hex(text.as_bytes()),
                        archived_at: now,
                        purged: false,
                    },
                );
            }
            let seg_path = dir.join(&segment);
            write_durably(&seg_path, &bytes)?;
            // Read back before the index names anything: the bytes on disk are
            // what the primary may be purged against.
            for (chunk, loc) in &locs {
                let back = inflate(&read_member_bytes(&seg_path, loc)?).unwrap_or_default();
                if sha256_hex(back.as_bytes()) != loc.sha256 {
                    let _ = fs::remove_file(&seg_path);
                    return Err(archive_error(format!(
                        "chunk {chunk} of {ns}/{log_id} did not read back as written; segment {segment} discarded"
                    )));
                }
            }
            let idx = SegmentIndex {
                ns: ns.to_owned(),
                log_id: log_id.to_owned(),
                chunks: locs.clone(),
            };
            let idx_text = serde_json::to_vec(&idx)?;
            write_durably(&dir.join(index_name(&segment)), &idx_text)?;
            let mut index = self.lock();
            let log = index
                .logs
                .entry((ns.to_owned(), log_id.to_owned()))
                .or_default();
            for (chunk, loc) in locs {
                log.chunks.insert(chunk, loc);
                admitted.push(chunk);
            }
        }
        Ok(admitted)
    }

    /// The raw text of `chunk`, or `None` when the archive does not hold it
    /// **or its bytes no longer hash to what the index recorded** (said at
    /// `error`): a damaged copy is absent, never served.
    ///
    /// # Errors
    /// An I/O failure other than damage (the segment file missing, unreadable).
    pub fn read_chunk(
        &self,
        ns: &str,
        log_id: &str,
        chunk: u64,
    ) -> Result<Option<String>, StoreError> {
        let Some(loc) = self.holds(ns, log_id, chunk) else {
            return Ok(None);
        };
        let path = self.log_dir(ns, log_id).join(&loc.segment);
        let bytes = read_member_bytes(&path, &loc)?;
        // Damage — a member that no longer inflates, or inflates to bytes that
        // do not hash to the index — is absence, never an error a caller might
        // retry into and never a served chunk.
        let Ok(text) = inflate(&bytes) else {
            tracing::error!(
                ns,
                log_id,
                chunk,
                segment = %path.display(),
                "ledger archive: chunk no longer inflates; treated as absent"
            );
            return Ok(None);
        };
        if sha256_hex(text.as_bytes()) != loc.sha256 {
            tracing::error!(
                ns,
                log_id,
                chunk,
                segment = %path.display(),
                "ledger archive: chunk does not hash to its index; treated as absent"
            );
            return Ok(None);
        }
        Ok(Some(text))
    }

    /// Record that the primary has been purged of `chunks`, in the index and
    /// in every affected segment's index file.
    ///
    /// # Errors
    /// An index file cannot be rewritten.
    pub fn mark_purged(&self, ns: &str, log_id: &str, chunks: &[u64]) -> Result<(), StoreError> {
        let dir = self.log_dir(ns, log_id);
        let mut touched: BTreeMap<String, BTreeMap<u64, ChunkLoc>> = BTreeMap::new();
        {
            let mut index = self.lock();
            let Some(log) = index.logs.get_mut(&(ns.to_owned(), log_id.to_owned())) else {
                return Ok(());
            };
            for chunk in chunks {
                if let Some(loc) = log.chunks.get_mut(chunk) {
                    loc.purged = true;
                    touched.entry(loc.segment.clone()).or_default();
                }
            }
            for (segment, locs) in &mut touched {
                for (chunk, loc) in &log.chunks {
                    if &loc.segment == segment {
                        locs.insert(*chunk, loc.clone());
                    }
                }
            }
        }
        for (segment, locs) in touched {
            let idx = SegmentIndex {
                ns: ns.to_owned(),
                log_id: log_id.to_owned(),
                chunks: locs,
            };
            write_durably(&dir.join(index_name(&segment)), &serde_json::to_vec(&idx)?)?;
        }
        Ok(())
    }
}

fn read_dir(dir: &Path) -> Result<Vec<PathBuf>, StoreError> {
    let entries =
        fs::read_dir(dir).map_err(|e| archive_error(format!("read dir {}: {e}", dir.display())))?;
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| archive_error(format!("read dir {}: {e}", dir.display())))?;
        out.push(entry.path());
    }
    out.sort();
    Ok(out)
}

/// The bytes of one gzip member at `loc` of the segment at `path` (an I/O
/// failure is an error; what the bytes mean is [`inflate`]'s question).
fn read_member_bytes(path: &Path, loc: &ChunkLoc) -> Result<Vec<u8>, StoreError> {
    let mut file =
        fs::File::open(path).map_err(|e| archive_error(format!("open {}: {e}", path.display())))?;
    file.seek(SeekFrom::Start(loc.offset))
        .map_err(|e| archive_error(format!("seek {}: {e}", path.display())))?;
    let mut member = vec![0u8; usize::try_from(loc.len).unwrap_or(usize::MAX)];
    file.read_exact(&mut member)
        .map_err(|e| archive_error(format!("read {}: {e}", path.display())))?;
    Ok(member)
}

/// Inflate one gzip member to the chunk text it held.
fn inflate(member: &[u8]) -> std::io::Result<String> {
    let mut text = String::new();
    GzDecoder::new(member).read_to_string(&mut text)?;
    Ok(text)
}

/// The tiered read for one ledger key (J5 §10.3): a primary miss on a `log:`
/// key consults the archive. `primary` is what the backend found.
///
/// # Errors
/// An archive I/O failure.
pub fn tiered_text(
    archive: Option<&LedgerArchive>,
    ns: &str,
    key: &str,
    primary: Option<String>,
) -> Result<Option<String>, StoreError> {
    if primary.is_some() {
        return Ok(primary);
    }
    let Some(archive) = archive else {
        return Ok(None);
    };
    let Some((log_id, chunk)) = mush_core::parse_chunk_key(key) else {
        return Ok(None);
    };
    archive.read_chunk(ns, log_id.as_str(), chunk)
}

/// The tiered listing (J5 §10.3): every archived chunk of `ns` the primary
/// listing lacks is added from the archive; a chunk in both is served from the
/// primary, and reported when the two disagree.
///
/// # Errors
/// An archive I/O failure.
pub fn merge_archived(
    archive: Option<&LedgerArchive>,
    ns: &str,
    ledger: &mut Vec<(String, String)>,
) -> Result<(), StoreError> {
    let Some(archive) = archive else {
        return Ok(());
    };
    let present: std::collections::HashMap<&str, &str> = ledger
        .iter()
        .map(|(k, t)| (k.as_str(), t.as_str()))
        .collect();
    let mut added: Vec<(String, String)> = Vec::new();
    for log_id in archive.logs_of(ns) {
        for (chunk, loc) in archive.chunks_of(ns, &log_id) {
            let key = mush_core::chunk_key(&log_id, chunk);
            if let Some(primary) = present.get(key.as_str()) {
                if sha256_hex(primary.as_bytes()) != loc.sha256 {
                    tracing::warn!(
                        ns,
                        log_id,
                        chunk,
                        "ledger archive: the primary's copy of a chunk differs from the archive's; the primary is served"
                    );
                }
                continue;
            }
            if let Some(text) = archive.read_chunk(ns, &log_id, chunk)? {
                added.push((key.to_string(), text));
            }
        }
    }
    ledger.extend(added);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk_text(chunk: u64, first_seq: u64, n: u64) -> String {
        let entries: Vec<String> = (first_seq..first_seq + n)
            .map(|seq| format!(r#"{{"seq":{seq},"value":{{"n":{seq}}}}}"#))
            .collect();
        format!(
            r#"{{"chunk":{chunk},"sealed":true,"entries":[{}]}}"#,
            entries.join(",")
        )
    }

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rustmush_archive_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn chunks_round_trip_through_segments_and_reopen() {
        let root = temp_root("roundtrip");
        let archive = LedgerArchive::open(&root).unwrap();
        let chunks: Vec<(u64, String)> = (0..5).map(|c| (c, chunk_text(c, c * 3, 3))).collect();
        let admitted = archive
            .store_chunks("economy", "journal", chunks.clone(), 1_000, 2)
            .unwrap();
        assert_eq!(admitted, vec![0, 1, 2, 3, 4]);
        for (c, text) in &chunks {
            assert_eq!(
                archive
                    .read_chunk("economy", "journal", *c)
                    .unwrap()
                    .as_deref(),
                Some(text.as_str())
            );
        }
        assert!(
            archive
                .read_chunk("economy", "journal", 9)
                .unwrap()
                .is_none()
        );
        let stats = archive.stats();
        assert_eq!((stats.chunks, stats.purged), (5, 0));
        assert!(stats.compressed_bytes < stats.raw_bytes);
        // Three segments of at most two chunks, each with its index.
        let dir = archive.log_dir("economy", "journal");
        let names: Vec<String> = read_dir(&dir)
            .unwrap()
            .iter()
            .filter_map(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(ToOwned::to_owned)
            })
            .collect();
        assert_eq!(names.iter().filter(|n| n.ends_with(".seg")).count(), 3);
        assert_eq!(names.iter().filter(|n| n.ends_with(".idx.json")).count(), 3);
        // Idempotent: a re-copy admits nothing.
        assert!(
            archive
                .store_chunks("economy", "journal", chunks.clone(), 2_000, 2)
                .unwrap()
                .is_empty()
        );
        // Purge marks survive a reopen.
        archive
            .mark_purged("economy", "journal", &[0, 1, 2])
            .unwrap();
        drop(archive);
        let reopened = LedgerArchive::open(&root).unwrap();
        let stats = reopened.stats();
        assert_eq!((stats.chunks, stats.purged), (5, 3));
        assert!(reopened.holds("economy", "journal", 1).unwrap().purged);
        assert!(!reopened.holds("economy", "journal", 4).unwrap().purged);
        assert_eq!(
            reopened
                .read_chunk("economy", "journal", 4)
                .unwrap()
                .as_deref(),
            Some(chunks[4].1.as_str())
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_damaged_segment_reads_as_absent_and_an_orphan_is_swept() {
        let root = temp_root("damage");
        let archive = LedgerArchive::open(&root).unwrap();
        archive
            .store_chunks("s", "x", vec![(0, chunk_text(0, 0, 2))], 1, 256)
            .unwrap();
        let dir = archive.log_dir("s", "x");
        let seg = dir.join(segment_name(0, 0));
        let mut bytes = fs::read(&seg).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        fs::write(&seg, bytes).unwrap();
        assert!(
            archive.read_chunk("s", "x", 0).unwrap().is_none(),
            "damage is absence"
        );
        // An orphan segment (no index) and a stray temp file are swept at open.
        fs::write(dir.join(segment_name(7, 7)), b"garbage").unwrap();
        fs::write(dir.join("000000000009-000000000009.seg.tmp"), b"half").unwrap();
        drop(archive);
        let reopened = LedgerArchive::open(&root).unwrap();
        assert!(reopened.holds("s", "x", 7).is_none());
        assert!(!dir.join(segment_name(7, 7)).exists());
        assert!(!dir.join("000000000009-000000000009.seg.tmp").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn tiered_helpers_prefer_the_primary_and_fill_from_the_archive() {
        let root = temp_root("tier");
        let archive = LedgerArchive::open(&root).unwrap();
        let c0 = chunk_text(0, 0, 2);
        let c1 = chunk_text(1, 2, 2);
        archive
            .store_chunks(
                "economy",
                "journal",
                vec![(0, c0.clone()), (1, c1.clone())],
                1,
                256,
            )
            .unwrap();
        // A primary hit is served as is; a miss on an archived key fills in.
        assert_eq!(
            tiered_text(
                Some(&archive),
                "economy",
                "log:journal:0",
                Some("primary".into())
            )
            .unwrap(),
            Some("primary".to_string())
        );
        assert_eq!(
            tiered_text(Some(&archive), "economy", "log:journal:1", None).unwrap(),
            Some(c1.clone())
        );
        assert!(
            tiered_text(Some(&archive), "economy", "log:journal:5", None)
                .unwrap()
                .is_none()
        );
        assert!(
            tiered_text(Some(&archive), "economy", "account:1", None)
                .unwrap()
                .is_none()
        );
        assert!(
            tiered_text(None, "economy", "log:journal:1", None)
                .unwrap()
                .is_none()
        );
        // The listing merge adds what the primary lacks, keeps what it has.
        let mut ledger = vec![("log:journal:0".to_string(), "different".to_string())];
        merge_archived(Some(&archive), "economy", &mut ledger).unwrap();
        ledger.sort();
        assert_eq!(
            ledger,
            vec![
                ("log:journal:0".to_string(), "different".to_string()),
                ("log:journal:1".to_string(), c1),
            ]
        );
        let _ = fs::remove_dir_all(&root);
        let _ = c0;
    }

    #[test]
    fn components_are_directory_safe() {
        assert_eq!(safe_component("journal"), "journal");
        assert_eq!(safe_component("scene:42/x"), "scene%3A42%2Fx");
        assert_eq!(segment_name(3, 40), "000000000003-000000000040.seg");
        assert_eq!(
            index_name("000000000003-000000000040.seg"),
            "000000000003-000000000040.idx.json"
        );
    }
}

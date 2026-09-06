//! **The economy checkpoint** (J3, doc 12 §5.2–§5.5): a persisted image of
//! the journal fold at a covered sequence, validated by hash, under the same
//! directional rule as every account document — **never authority**. Boot
//! seeds the fold from a valid checkpoint and replays only the tail; an
//! invalid one is ignored, said at `error`, and the journal is replayed whole.
//!
//! ## Documents
//!
//! - **`checkpoint:{seq}`**, the head: the covered sequence and chunk, when it
//!   was written, the part list (`key`, SHA-256, sequence range), the
//!   `state_hash`, and the **mutable body** — the [`FoldState`] at the covered
//!   sequence plus the debit-window tails and the chunk-to-first-sequence map.
//! - **`checkpoint:{seq}:idem:{n}`**, the immutable parts: the idempotency
//!   records and the transaction refs for one range of entries, compactly.
//!   A part covering a range never changes, so a later checkpoint re-lists the
//!   parts it inherits and writes only the new range; the head's part list is
//!   what makes them one checkpoint.
//!
//! Written parts first, head last; a crash between leaves parts nobody names,
//! swept by the next write's GC together with every head older than the one
//! the new head superseded.
//!
//! ## The write-time drift check (§5.3)
//!
//! The live store carries the fold as a **journal-authority shadow**
//! ([`crate::module::EconomyStore::fold`]), maintained per post by the one
//! function boot replays with. Before a checkpoint is written, every account,
//! title and reservation projection is compared with the shadow; a mismatch
//! writes nothing, says so at `error`, and **freezes the economy** — the same
//! money-safety rule boot applies to drift, now at runtime.
//!
//! ## What the drift detector loses (§5.5, D-12-2)
//!
//! A boot from a checkpoint cannot visit sequences at or before the covered
//! one. A document claiming such a sequence is judged only when that sequence
//! is the last the fold touched for it (then the checkpoint's value *is* the
//! value at the claim); otherwise it **lags** and is healed to the journal-
//! derived value **without** a drift or "never appears at" verdict, said at
//! `info`. The owner accepted that loss with the write-time check as its
//! condition.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{
    BOOT_MANIFEST_FORMAT, BOOT_MANIFEST_KEY, BootManifest, Ctx, LedgerHistory, ModuleWrite,
    canonical_json_bytes,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::api::{delete_key, push_op};
use crate::fold::FoldState;
use crate::ids::{AccountId, TransactionId};
use crate::module::{EconomyStore, JOURNAL_LOG, TxRef, WindowTail};
use crate::record::{IdempotencyKey, IdempotencyRecord};
use crate::snapshot::EconomySnapshot;

/// The document format this code writes and accepts.
pub const CHECKPOINT_FORMAT: u32 = 1;
/// Domain separation for [`state_hash`].
pub const CHECKPOINT_HASH_DOMAIN: &str = "rustmush.economy.checkpoint.v1";
/// The most rows one immutable part carries (~3 MB of JSON).
pub const MAX_PART_ROWS: usize = 25_000;
/// How many journal entries past the last checkpoint start the next one by
/// default (`[economy] checkpoint_every_entries`; 0 disables).
pub const DEFAULT_CHECKPOINT_EVERY_ENTRIES: u64 = 10_000;

/// The head's key: `checkpoint:{seq}`.
#[must_use]
pub fn head_key(seq: u64) -> SmolStr {
    SmolStr::new(format!("checkpoint:{seq}"))
}

/// A part's key: `checkpoint:{seq}:idem:{n}`.
#[must_use]
pub fn part_key(seq: u64, n: usize) -> SmolStr {
    SmolStr::new(format!("checkpoint:{seq}:idem:{n}"))
}

/// Whether `key` is any checkpoint document (head or part).
#[must_use]
pub fn is_checkpoint_key(key: &str) -> bool {
    key.starts_with("checkpoint:")
}

pub(crate) fn parse_head_key(key: &str) -> Option<u64> {
    let rest = key.strip_prefix("checkpoint:")?;
    if rest.contains(':') {
        return None;
    }
    rest.parse().ok()
}

pub(crate) fn parse_part_key(key: &str) -> Option<(u64, usize)> {
    let rest = key.strip_prefix("checkpoint:")?;
    let mut it = rest.split(':');
    let seq: u64 = it.next()?.parse().ok()?;
    if it.next()? != "idem" {
        return None;
    }
    let n: usize = it.next()?.parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((seq, n))
}

/// One immutable part as the head lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartRef {
    /// The part's document key.
    pub key: SmolStr,
    /// SHA-256 (hex) of the part document's canonical JSON.
    pub sha256: String,
    /// The first sequence the part covers.
    pub seq_lo: u64,
    /// The last sequence the part covers.
    pub seq_hi: u64,
    /// How many rows it carries.
    pub rows: usize,
    /// The highest transaction id it carries.
    pub max_txid: u64,
}

/// The mutable body: the fold at the covered sequence and the two maps boot
/// keeps beside it.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckpointBody {
    /// The journal fold at `covered_seq`.
    pub fold: FoldState,
    /// The rolling-window debit tails (live-filtered per D4).
    pub debit_window: BTreeMap<AccountId, WindowTail>,
    /// Chunk index → first sequence, for every chunk at or below the covered one.
    pub chunk_firsts: BTreeMap<u64, u64>,
}

/// The head document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CheckpointHead {
    /// [`CHECKPOINT_FORMAT`].
    pub format: u32,
    /// The last journal sequence folded into the body.
    pub covered_seq: u64,
    /// The chunk holding `covered_seq` (a boot may skip every chunk below it).
    pub covered_chunk: u64,
    /// When it was written (wall-clock seconds).
    pub written_at: i64,
    /// The highest transaction id the parts carry.
    pub max_txid: u64,
    /// The parts, in sequence order.
    pub parts: Vec<PartRef>,
    /// [`state_hash`] over the body and the parts.
    pub state_hash: String,
    /// The mutable body.
    pub body: CheckpointBody,
}

/// One row of a part: an idempotency record and a transaction ref, compactly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdemRow {
    /// The idempotency key's namespace.
    pub ns: SmolStr,
    /// The idempotency key.
    pub key: SmolStr,
    /// The plan hash it was posted under, hex.
    pub plan_hash: String,
    /// The transaction it produced.
    pub tx: TransactionId,
    /// When it was posted.
    pub posted_at: i64,
    /// The journal sequence of that transaction.
    pub seq: u64,
}

/// An immutable part document.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IdemPart {
    /// [`CHECKPOINT_FORMAT`].
    pub format: u32,
    /// The first sequence covered.
    pub seq_lo: u64,
    /// The last sequence covered.
    pub seq_hi: u64,
    /// The rows, in sequence order.
    pub rows: Vec<IdemRow>,
}

/// What the store remembers about the checkpoint it last wrote or booted from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CheckpointMeta {
    /// The last sequence it covers.
    pub covered_seq: u64,
    /// Its head's key.
    pub head_key: SmolStr,
    /// Its parts.
    pub parts: Vec<PartRef>,
    /// The highest transaction id its parts carry.
    pub max_txid: u64,
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn unhex32(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, chunk) in text.as_bytes().chunks(2).enumerate() {
        let s = std::str::from_utf8(chunk).ok()?;
        out[i] = u8::from_str_radix(s, 16).ok()?;
    }
    Some(out)
}

/// SHA-256 (hex) of a document's canonical JSON; `None` when it cannot be
/// canonically encoded.
#[must_use]
pub(crate) fn canonical_hash(value: &serde_json::Value) -> Option<String> {
    canonical_json_bytes(value)
        .ok()
        .map(|bytes| hex(&Sha256::digest(bytes)))
}

/// The head's `state_hash`: the domain, the body's canonical JSON, and every
/// part's hash in order.
#[must_use]
pub(crate) fn state_hash(body: &serde_json::Value, parts: &[PartRef]) -> Option<String> {
    let body_bytes = canonical_json_bytes(body).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(CHECKPOINT_HASH_DOMAIN.as_bytes());
    hasher.update(body_bytes);
    for part in parts {
        hasher.update(part.sha256.as_bytes());
    }
    Some(hex(&hasher.finalize()))
}

/// The write-time drift check (§5.3): every account, title and reservation
/// projection against the journal-authority shadow. Empty means clean; each
/// entry names one disagreement.
#[must_use]
pub(crate) fn drift_between(fold: &FoldState, snapshot: &EconomySnapshot) -> Vec<String> {
    let mut out = Vec::new();
    for account in snapshot.accounts() {
        let id = account.id;
        let running = fold.running.get(&id).copied().unwrap_or(0);
        if Amount::from_widened(running) != Some(account.posted_balance) {
            out.push(format!(
                "account #{}: balance {} vs journal {running}",
                id,
                account.posted_balance.minor()
            ));
        }
        let touch = fold.last_touch.get(&id).copied().unwrap_or(0);
        if account.last_journal_seq != touch {
            out.push(format!(
                "account #{}: last_journal_seq {} vs journal {touch}",
                id, account.last_journal_seq
            ));
        }
        let held = fold.running_held.get(&id).copied().unwrap_or(0);
        if Amount::from_widened(held) != Some(account.held_amount) {
            out.push(format!(
                "account #{}: held {} vs journal {held}",
                id,
                account.held_amount.minor()
            ));
        }
        let hold_touch = fold.last_hold_touch.get(&id).copied().unwrap_or(0);
        if account.last_hold_seq != hold_touch {
            out.push(format!(
                "account #{}: last_hold_seq {} vs journal {hold_touch}",
                id, account.last_hold_seq
            ));
        }
    }
    for (object, journal) in &fold.journal_assets {
        match snapshot.asset(*object) {
            Some(live) if crate::boot::journal_fields_eq(live, journal) => {}
            Some(_) => out.push(format!(
                "asset #{}: projection differs from the journal",
                object.0
            )),
            None => out.push(format!("asset #{}: journaled but not projected", object.0)),
        }
    }
    for (id, journal) in &fold.journal_holds {
        match snapshot.hold(*id) {
            Some(live) if live == journal => {}
            Some(_) => out.push(format!(
                "hold #{}: projection differs from the journal",
                id.0
            )),
            None => out.push(format!("hold #{}: journaled but not projected", id.0)),
        }
    }
    out
}

/// Write a checkpoint if the journal has grown past the cadence since the
/// last one — from the sweep, never the post path. Refuses (and freezes) on
/// drift; writes nothing while frozen or disabled.
pub(crate) fn maybe_write(ctx: &mut Ctx<'_>, now: i64) {
    let (every, next_uncovered, previous, frozen, journal_len) = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        (
            store.config.checkpoint_every_entries,
            store.checkpoint.as_ref().map_or(0, |c| c.covered_seq + 1),
            store.checkpoint.clone(),
            store.frozen,
            store.journal_len(),
        )
    };
    if every == 0 || frozen || journal_len.saturating_sub(next_uncovered) < every {
        return;
    }
    let drift = {
        let store = ctx.w.module_state::<EconomyStore>().expect("checked above");
        drift_between(&store.fold, &store.snapshot)
    };
    if !drift.is_empty() {
        tracing::error!(
            mismatches = drift.len(),
            first = %drift[0],
            "economy: the projections drift from the journal-authority shadow; no checkpoint written, freezing"
        );
        crate::api::set_economy_frozen(ctx, true);
        return;
    }
    let covered_seq = journal_len - 1;
    let (built, manifest_floor) = {
        let store = ctx.w.module_state::<EconomyStore>().expect("checked above");
        if store.fold.covered_seq != Some(covered_seq) {
            tracing::error!(
                fold = ?store.fold.covered_seq,
                journal = covered_seq,
                "economy: the fold is not at the journal head; no checkpoint written"
            );
            return;
        }
        // J4 (doc 12 §5.6): the chunk holding the RAM tail's floor at this
        // head — every chunk below it is covered by the head and outside the
        // tail, so the loader may skip it. Conservative by construction: the
        // tail's floor only moves up as posts follow.
        let tail_max = u64::try_from(store.tx_tail_max()).unwrap_or(u64::MAX);
        let tail_floor_seq = covered_seq.saturating_add(1).saturating_sub(tail_max);
        let floor = store.chunk_of_seq(tail_floor_seq).unwrap_or(0);
        (build(store, previous.as_ref(), covered_seq, now), floor)
    };
    let Some(built) = built else {
        return;
    };
    let BuiltCheckpoint {
        head_key: new_head_key,
        head,
        part_docs,
        meta,
    } = built;
    // Parts first, the head last; then the GC of what neither the new head nor
    // the one it supersedes names.
    for (key, value) in part_docs {
        push_op(ctx.out, ModuleWrite::JsonSet { key, value });
    }
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: new_head_key.clone(),
            value: head,
        },
    );
    // J4: the boot manifest beside the head, for the loader.
    let manifest = BootManifest {
        format: BOOT_MANIFEST_FORMAT,
        skip_log_chunks_below: [(SmolStr::new(JOURNAL_LOG), manifest_floor)]
            .into_iter()
            .collect(),
    };
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: SmolStr::new(BOOT_MANIFEST_KEY),
            value: serde_json::to_value(&manifest).expect("a boot manifest serializes"),
        },
    );
    let mut keep: BTreeSet<SmolStr> = BTreeSet::new();
    keep.insert(new_head_key.clone());
    for p in &meta.parts {
        keep.insert(p.key.clone());
    }
    if let Some(prev) = &previous {
        keep.insert(prev.head_key.clone());
        for p in &prev.parts {
            keep.insert(p.key.clone());
        }
    }
    let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
        return;
    };
    let stale: Vec<SmolStr> = store
        .checkpoint_keys
        .iter()
        .filter(|k| !keep.contains(*k))
        .cloned()
        .collect();
    store.checkpoint_keys = keep;
    store.checkpoint = Some(meta);
    tracing::info!(
        covered_seq,
        parts = store.checkpoint.as_ref().map_or(0, |m| m.parts.len()),
        swept = stale.len(),
        "economy: checkpoint written"
    );
    for key in stale {
        delete_key(ctx.out, key);
    }
}

struct BuiltCheckpoint {
    head_key: SmolStr,
    head: serde_json::Value,
    part_docs: Vec<(SmolStr, serde_json::Value)>,
    meta: CheckpointMeta,
}

/// Build the next checkpoint's documents from the live store: the parts for
/// every transaction past the previous checkpoint's highest id, the head with
/// the inherited parts re-listed.
fn build(
    store: &EconomyStore,
    previous: Option<&CheckpointMeta>,
    covered_seq: u64,
    now: i64,
) -> Option<BuiltCheckpoint> {
    let mut rows: Vec<IdemRow> = Vec::new();
    let start = previous.map_or(0, |p| p.max_txid.saturating_add(1));
    let first_ever = previous.is_none();
    for (id, r) in store.tx_index.range(TransactionId(start)..) {
        if !first_ever && previous.is_some_and(|p| id.0 <= p.max_txid) {
            continue;
        }
        let Some(record) = store.snapshot.idempotency(&r.idempotency) else {
            tracing::error!(
                transaction = id.0,
                "economy: a transaction has no idempotency record; no checkpoint written"
            );
            return None;
        };
        rows.push(IdemRow {
            ns: r.idempotency.namespace.clone(),
            key: r.idempotency.key.clone(),
            plan_hash: hex(&record.plan_hash),
            tx: *id,
            posted_at: r.posted_at,
            seq: r.seq,
        });
    }
    let mut parts: Vec<PartRef> = previous.map(|p| p.parts.clone()).unwrap_or_default();
    let mut part_docs: Vec<(SmolStr, serde_json::Value)> = Vec::new();
    let inherited = parts.len();
    for (i, run) in rows.chunks(MAX_PART_ROWS).enumerate() {
        let n = inherited + i;
        let part = IdemPart {
            format: CHECKPOINT_FORMAT,
            seq_lo: run[0].seq,
            seq_hi: run[run.len() - 1].seq,
            rows: run.to_vec(),
        };
        let value = serde_json::to_value(&part).ok()?;
        let sha256 = canonical_hash(&value)?;
        let key = part_key(covered_seq, n);
        parts.push(PartRef {
            key: key.clone(),
            sha256,
            seq_lo: part.seq_lo,
            seq_hi: part.seq_hi,
            rows: run.len(),
            max_txid: run[run.len() - 1].tx.0,
        });
        part_docs.push((key, value));
    }
    let max_txid = parts.last().map_or(0, |p| p.max_txid);
    let body = CheckpointBody {
        fold: store.fold.clone(),
        debit_window: store.debit_window.clone(),
        chunk_firsts: store.chunk_firsts.clone(),
    };
    let body_value = serde_json::to_value(&body).ok()?;
    let state_hash = state_hash(&body_value, &parts)?;
    let head = CheckpointHead {
        format: CHECKPOINT_FORMAT,
        covered_seq,
        covered_chunk: store.chunk_of_seq(covered_seq).unwrap_or(0),
        written_at: now,
        max_txid,
        parts: parts.clone(),
        state_hash,
        body,
    };
    let head_value = serde_json::to_value(&head).ok()?;
    Some(BuiltCheckpoint {
        head_key: head_key(covered_seq),
        head: head_value,
        part_docs,
        meta: CheckpointMeta {
            covered_seq,
            head_key: head_key(covered_seq),
            parts,
            max_txid,
        },
    })
}

/// A checkpoint boot may seed from.
pub(crate) struct ValidCheckpoint {
    /// The head, decoded.
    pub head: CheckpointHead,
    /// Its key.
    pub head_key: SmolStr,
    /// Every part's rows, in sequence order.
    pub rows: Vec<IdemRow>,
}

impl ValidCheckpoint {
    /// The idempotency record and the transaction ref one row carries.
    pub(crate) fn record_of(row: &IdemRow) -> Option<(IdempotencyRecord, TxRef)> {
        let plan_hash = unhex32(&row.plan_hash)?;
        let key = IdempotencyKey::new(row.ns.clone(), row.key.clone());
        Some((
            IdempotencyRecord {
                idempotency: key.clone(),
                plan_hash,
                transaction: row.tx,
                posted_at: row.posted_at,
            },
            TxRef {
                seq: row.seq,
                idempotency: key,
                posted_at: row.posted_at,
            },
        ))
    }
}

/// Select the newest **valid** checkpoint among the boot documents, with a
/// note for every one rejected (said at `error` by the caller). A checkpoint
/// is valid when its key agrees with its covered sequence, the sequence is at
/// or below the journal head, its covered chunk exists, every part is present
/// and hashes to what the head lists, every part decodes and covers what it
/// claims, and the `state_hash` recomputes.
pub(crate) fn select(
    kv: &[(SmolStr, serde_json::Value)],
    journal_max: Option<u64>,
    history: &LedgerHistory,
) -> (Option<ValidCheckpoint>, Vec<String>) {
    let mut notes = Vec::new();
    let mut heads: Vec<(u64, SmolStr, CheckpointHead)> = Vec::new();
    let mut parts: BTreeMap<&str, &serde_json::Value> = BTreeMap::new();
    for (key, value) in kv {
        if let Some(seq) = parse_head_key(key) {
            match serde_json::from_value::<CheckpointHead>(value.clone()) {
                Ok(head) if head.format == CHECKPOINT_FORMAT => {
                    heads.push((seq, key.clone(), head))
                }
                Ok(head) => notes.push(format!(
                    "{key}: format {} is not {CHECKPOINT_FORMAT}",
                    head.format
                )),
                Err(error) => notes.push(format!("{key}: undecodable head: {error}")),
            }
        } else if parse_part_key(key).is_some() {
            parts.insert(key.as_str(), value);
        }
    }
    heads.sort_by_key(|(seq, _, _)| std::cmp::Reverse(*seq));
    let chunks: BTreeSet<u64> = history.chunk_docs(JOURNAL_LOG).map(|(c, _)| c).collect();
    for (seq, key, head) in heads {
        match validate(&head, seq, &parts, journal_max, &chunks) {
            Ok(rows) => {
                return (
                    Some(ValidCheckpoint {
                        head,
                        head_key: key,
                        rows,
                    }),
                    notes,
                );
            }
            Err(reason) => notes.push(format!("{key}: {reason}")),
        }
    }
    (None, notes)
}

fn validate(
    head: &CheckpointHead,
    seq: u64,
    parts: &BTreeMap<&str, &serde_json::Value>,
    journal_max: Option<u64>,
    chunks: &BTreeSet<u64>,
) -> Result<Vec<IdemRow>, String> {
    if head.covered_seq != seq {
        return Err(format!(
            "covers {} under a key naming {seq}",
            head.covered_seq
        ));
    }
    match journal_max {
        Some(max) if head.covered_seq <= max => {}
        _ => {
            return Err(format!(
                "covers {} past the journal head {journal_max:?}",
                head.covered_seq
            ));
        }
    }
    if head.body.fold.covered_seq != Some(head.covered_seq) {
        return Err(format!(
            "its fold covers {:?}, not {}",
            head.body.fold.covered_seq, head.covered_seq
        ));
    }
    if !chunks.contains(&head.covered_chunk) {
        return Err(format!(
            "covered chunk {} is not in the journal",
            head.covered_chunk
        ));
    }
    let mut rows: Vec<IdemRow> = Vec::new();
    let mut expected_lo: Option<u64> = None;
    for part in &head.parts {
        let Some(value) = parts.get(part.key.as_str()) else {
            return Err(format!("part {} is missing", part.key));
        };
        let hash =
            canonical_hash(value).ok_or_else(|| format!("part {} cannot be hashed", part.key))?;
        if hash != part.sha256 {
            return Err(format!(
                "part {} does not hash to what the head lists",
                part.key
            ));
        }
        let decoded: IdemPart = serde_json::from_value((*value).clone())
            .map_err(|e| format!("part {} is undecodable: {e}", part.key))?;
        if decoded.format != CHECKPOINT_FORMAT
            || decoded.seq_lo != part.seq_lo
            || decoded.seq_hi != part.seq_hi
            || decoded.rows.len() != part.rows
        {
            return Err(format!(
                "part {} does not cover what the head lists",
                part.key
            ));
        }
        if let Some(lo) = expected_lo
            && decoded.seq_lo < lo
        {
            return Err(format!("part {} overlaps the part before it", part.key));
        }
        expected_lo = Some(decoded.seq_hi.saturating_add(1));
        rows.extend(decoded.rows);
    }
    let body_value = serde_json::to_value(&head.body).map_err(|e| format!("body: {e}"))?;
    let recomputed = state_hash(&body_value, &head.parts).ok_or("the body cannot be hashed")?;
    if recomputed != head.state_hash {
        return Err("the state hash does not recompute".to_owned());
    }
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_parse_and_nothing_else_does() {
        assert_eq!(parse_head_key("checkpoint:42"), Some(42));
        assert_eq!(parse_head_key("checkpoint:42:idem:0"), None);
        assert_eq!(parse_part_key("checkpoint:42:idem:3"), Some((42, 3)));
        assert_eq!(parse_part_key("checkpoint:42:idem:3:x"), None);
        assert_eq!(parse_part_key("checkpoint:42"), None);
        assert!(is_checkpoint_key("checkpoint:1"));
        assert!(!is_checkpoint_key("account:1"));
        assert_eq!(head_key(7).as_str(), "checkpoint:7");
        assert_eq!(part_key(7, 2).as_str(), "checkpoint:7:idem:2");
        assert_eq!(unhex32(&"ab".repeat(32)), Some([0xab; 32]));
        assert_eq!(unhex32("zz"), None);
    }

    /// The write-time drift check: a projection that disagrees with the
    /// shadow on any journal-authority field is named; a clean one is not.
    #[test]
    fn drift_between_names_every_disagreement() {
        use crate::record::{Account, AccountStatus, CreditPolicy};
        let mut snapshot = EconomySnapshot::new();
        let account = |id: u64, balance: i64, seq: u64| Account {
            id: AccountId(id),
            owner: mush_org::PrincipalId::Object(mush_core::ObjId(1)),
            instrument: crate::ids::InstrumentId::new("gold").expect("instrument"),
            scope: None,
            name: "a".into(),
            status: AccountStatus::Active,
            posted_balance: Amount::from_minor(balance),
            held_amount: Amount::from_minor(0),
            credit: CreditPolicy::forbid(),
            last_journal_seq: seq,
            revision: 1,
            approval: None,
            last_hold_seq: 0,
        };
        snapshot.put_account(account(1, 5, 2));
        let mut fold = FoldState::default();
        fold.running.insert(AccountId(1), 5);
        fold.last_touch.insert(AccountId(1), 2);
        assert!(drift_between(&fold, &snapshot).is_empty());
        fold.running.insert(AccountId(1), 6);
        let drift = drift_between(&fold, &snapshot);
        assert_eq!(drift.len(), 1, "{drift:?}");
        assert!(drift[0].contains("balance"), "{drift:?}");
        fold.last_touch.insert(AccountId(1), 3);
        assert_eq!(drift_between(&fold, &snapshot).len(), 2);
    }

    #[test]
    fn the_state_hash_is_domain_separated_and_covers_the_parts() {
        let body = serde_json::json!({ "a": 1 });
        let none = state_hash(&body, &[]).unwrap();
        let one = state_hash(
            &body,
            &[PartRef {
                key: "checkpoint:1:idem:0".into(),
                sha256: "00".repeat(32),
                seq_lo: 0,
                seq_hi: 1,
                rows: 2,
                max_txid: 1,
            }],
        )
        .unwrap();
        assert_ne!(none, one);
        assert_ne!(none, canonical_hash(&body).unwrap(), "the domain matters");
        assert_eq!(
            none,
            state_hash(&serde_json::json!({ "a": 1 }), &[]).unwrap()
        );
    }
}

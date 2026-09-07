//! Every idempotency key on disk, forever (doc 13 §2.1, W-D4), and the slow
//! lookup the filter gates (§2.4, W-D1).
//!
//! A post writes **`idem:{h}`** — `h` the hex SHA-256 of `namespace ‖ 0x00 ‖
//! key` — in the same batch as its journal append, carrying the
//! [`IdempotencyRecord`] and the journal sequence. The documents are never
//! loaded at boot (the boot manifest names the prefix; the loader skips it)
//! and never deleted (D-12-5). They are read one at a time, off the world
//! loop, through the store-read seam, when the gate cannot decide a key from
//! RAM: the answer lands in the store's cold cache and the same request a
//! moment later is decided there.
//!
//! For a journal older than the cut the sweep **backfills** the documents at
//! its own pace (§2.6, W-D5), from the RAM index — which still holds every
//! key, because the window's eviction (W2) waits for the backfill to finish.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use smol_str::SmolStr;

use mush_core::{Ctx, ModuleWrite, StoreReadResult};

use crate::api::push_op;
use crate::ids::TransactionId;
use crate::module::{EconomyStore, PendingKeyRead};
use crate::record::{EconomicTransaction, IdempotencyKey, IdempotencyRecord};

/// The key prefix of every key document.
pub const IDEM_PREFIX: &str = "idem:";
/// [`IdemDoc::format`] when written by this code.
pub const IDEM_FORMAT: u32 = 1;
/// Key documents the sweep backfills per fire for a pre-cut journal.
pub const IDEM_BACKFILL_PER_SWEEP: usize = 2_000;
/// How long a cold-cache answer is kept (seconds): long past any retry's
/// "shortly", short enough that a key read once does not pin RAM.
pub const COLD_KEY_TTL_SECS: i64 = 600;

/// The document key of an idempotency key: `idem:{hex sha256(ns ‖ 0 ‖ key)}`.
#[must_use]
pub fn idem_key(key: &IdempotencyKey) -> SmolStr {
    let mut hasher = Sha256::new();
    hasher.update(key.namespace.as_bytes());
    hasher.update([0u8]);
    hasher.update(key.key.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(IDEM_PREFIX.len() + 64);
    out.push_str(IDEM_PREFIX);
    for b in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    SmolStr::new(out)
}

/// Whether a persisted key is a key document.
#[must_use]
pub fn is_idem_key(key: &str) -> bool {
    key.starts_with(IDEM_PREFIX)
}

/// The key document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdemDoc {
    /// [`IDEM_FORMAT`].
    pub format: u32,
    /// The key's namespace.
    pub ns: SmolStr,
    /// The key.
    pub key: SmolStr,
    /// The plan hash it was posted under, hex.
    pub plan_hash: String,
    /// The transaction it produced.
    pub tx: TransactionId,
    /// When it was posted (wall-clock seconds).
    pub posted_at: i64,
    /// The journal sequence of that transaction.
    pub seq: u64,
}

impl IdemDoc {
    /// The document a posted transaction writes.
    #[must_use]
    pub fn of(tx: &EconomicTransaction) -> IdemDoc {
        IdemDoc {
            format: IDEM_FORMAT,
            ns: tx.idempotency.namespace.clone(),
            key: tx.idempotency.key.clone(),
            plan_hash: hex(&tx.plan_hash),
            tx: tx.id,
            posted_at: tx.posted_at,
            seq: tx.journal_seq,
        }
    }

    /// The document for a record the backfill reads from the RAM index.
    #[must_use]
    pub fn from_record(record: &IdempotencyRecord, seq: u64) -> IdemDoc {
        IdemDoc {
            format: IDEM_FORMAT,
            ns: record.idempotency.namespace.clone(),
            key: record.idempotency.key.clone(),
            plan_hash: hex(&record.plan_hash),
            tx: record.transaction,
            posted_at: record.posted_at,
            seq,
        }
    }

    /// The idempotency key this document is for.
    #[must_use]
    pub fn idempotency(&self) -> IdempotencyKey {
        IdempotencyKey::new(self.ns.clone(), self.key.clone())
    }

    /// The record it carries; `None` for a document this code cannot read.
    #[must_use]
    pub fn record(&self) -> Option<IdempotencyRecord> {
        if self.format != IDEM_FORMAT {
            return None;
        }
        Some(IdempotencyRecord {
            idempotency: self.idempotency(),
            plan_hash: unhex32(&self.plan_hash)?,
            transaction: self.tx,
            posted_at: self.posted_at,
        })
    }
}

/// Persist a key document.
pub(crate) fn persist_idem(out: &mut mush_core::Outbox, doc: &IdemDoc) {
    push_op(
        out,
        ModuleWrite::JsonSet {
            key: idem_key(&doc.idempotency()),
            value: serde_json::to_value(doc).expect("a key document serializes"),
        },
    );
}

/// Ask the store for a key's document, off the world loop (W-D1): one read in
/// flight per key, keyed by the key and not the actor, because sweep-driven
/// callers post as the system. The read is the economy's whichever module's
/// command is posting (W-D8): the completion must route here.
pub(crate) fn request_key_read(ctx: &mut Ctx<'_>, key: &IdempotencyKey) {
    let now = ctx.w.now();
    {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        if store
            .pending_key_reads
            .get(key)
            .is_some_and(|p| now.saturating_sub(p.requested_at) < mush_core::STORE_READ_STALE_SECS)
        {
            return;
        }
    }
    let caller = ctx.module.replace(SmolStr::new_static(crate::module::NS));
    let requested = ctx.read_store(vec![idem_key(key)]);
    ctx.module = caller;
    match requested {
        Ok(correlation) => {
            if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
                store.pending_key_reads.insert(
                    key.clone(),
                    PendingKeyRead {
                        correlation,
                        requested_at: now,
                    },
                );
            }
        }
        Err(error) => {
            tracing::warn!(%error, "economy: the key document read could not be requested");
        }
    }
}

/// The completion of a keyed read: the answer — the document's record, or
/// nothing — goes into the cold cache under the key the read was for. A
/// failed or oversized read is forgotten, so the next attempt asks again.
pub(crate) fn complete_key_read(ctx: &mut Ctx<'_>, key: &IdempotencyKey, result: StoreReadResult) {
    let now = ctx.w.now();
    let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
        return;
    };
    store.pending_key_reads.remove(key);
    let wanted = idem_key(key);
    match result {
        StoreReadResult::Docs { docs, absent } => {
            if let Some((_, value)) = docs.iter().find(|(k, _)| *k == wanted) {
                match serde_json::from_value::<IdemDoc>(value.clone())
                    .ok()
                    .and_then(|d| d.record())
                {
                    Some(record) if record.idempotency == *key => {
                        store.note_cold_key(key.clone(), Some(record), now);
                    }
                    _ => {
                        tracing::error!(
                            key = %wanted,
                            "economy: a key document is unreadable or names another key; the key stays undecided"
                        );
                    }
                }
            } else if absent.contains(&wanted) {
                store.note_cold_key(key.clone(), None, now);
            } else {
                tracing::warn!(key = %wanted, "economy: a key read answered without the key");
            }
        }
        StoreReadResult::Failed(detail) => {
            tracing::warn!(key = %wanted, %detail, "economy: a key document read failed");
        }
        StoreReadResult::Oversized { bytes } => {
            tracing::error!(key = %wanted, bytes, "economy: a key document read was oversized");
        }
    }
}

/// The sweep's backfill of key documents for a journal older than the cut
/// (W-D5): up to [`IDEM_BACKFILL_PER_SWEEP`] transactions past the cursor,
/// their records read from the RAM index; done when the cursor passes the
/// newest id. The cursor and the flag ride in `meta`.
pub(crate) fn backfill(ctx: &mut Ctx<'_>) {
    let (docs, meta) = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
            return;
        };
        if store.idem_backfilled {
            return;
        }
        let mut docs: Vec<IdemDoc> = Vec::new();
        let mut last: Option<u64> = None;
        let mut taken = 0usize;
        for (id, r) in store
            .tx_index
            .range(TransactionId(store.idem_backfill_txid)..)
            .take(IDEM_BACKFILL_PER_SWEEP)
        {
            taken += 1;
            last = Some(id.0);
            match store.snapshot.idempotency(&r.idempotency) {
                Some(record) => docs.push(IdemDoc::from_record(record, r.seq)),
                None => tracing::error!(
                    transaction = id.0,
                    "economy: a transaction has no idempotency record; its key document is not backfilled"
                ),
            }
        }
        if let Some(last) = last {
            store.idem_backfill_txid = last.saturating_add(1);
        }
        if taken < IDEM_BACKFILL_PER_SWEEP {
            store.idem_backfilled = true;
            // Doc 13 W2: every key has its document; the window may evict.
            store.key_mode = crate::module::KeyMode::Windowed;
            tracing::info!(
                documents = store.idem_backfill_txid,
                "economy: the key documents are backfilled"
            );
        }
        (docs, store.meta_doc())
    };
    for doc in &docs {
        persist_idem(ctx.out, doc);
    }
    crate::api::persist_meta(ctx.out, meta);
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(out, "{b:02x}");
    }
    out
}

pub(crate) fn unhex32(text: &str) -> Option<[u8; 32]> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_document_key_is_the_hash_of_namespace_and_key() {
        let a = idem_key(&IdempotencyKey::new("t", "k"));
        let b = idem_key(&IdempotencyKey::new("t", "k"));
        let c = idem_key(&IdempotencyKey::new("tk", ""));
        assert_eq!(a, b);
        assert_ne!(a, c, "the separator keeps `t`+`k` apart from `tk`+``");
        assert!(is_idem_key(&a));
        assert_eq!(a.len(), IDEM_PREFIX.len() + 64);
        assert!(!is_idem_key("account:1"));
    }

    #[test]
    fn the_document_round_trips_its_record() {
        let record = IdempotencyRecord {
            idempotency: IdempotencyKey::new("t", "k"),
            plan_hash: [7u8; 32],
            transaction: TransactionId(9),
            posted_at: 1_700_000_000,
        };
        let doc = IdemDoc::from_record(&record, 42);
        let text = serde_json::to_string(&doc).unwrap();
        let back: IdemDoc = serde_json::from_str(&text).unwrap();
        assert_eq!(back.record(), Some(record));
        assert_eq!(back.seq, 42);
        let mut unknown = back.clone();
        unknown.format = 9;
        assert_eq!(unknown.record(), None);
        assert_eq!(unhex32("zz"), None);
    }
}

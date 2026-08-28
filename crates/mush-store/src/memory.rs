//! [`InMemoryStore`] — the complete in-process [`ObjectStore`] used by tests
//! and `mushd --repl`.
//!
//! Fidelity to the Redis implementation is the whole point:
//!
//! - documents are held **serialized** (JSON strings), so every write/load
//!   crosses the same serde path a Redis round trip does — the serde
//!   stability tests in the shared suite are real here, not tautologies;
//! - a [`WriteBatch`] applies all-or-nothing under one lock (serialization,
//!   the only fallible step, happens before any mutation);
//! - upserts land before deletes, so an id in both lists ends up deleted;
//! - module ops apply after the object writes, in queue order; module
//!   streams are capped `VecDeque`s (exact trim — see [`MemStream`]) with
//!   monotonic `"N-0"` ids;
//! - ban expiry mirrors Redis TTL: expired entries are pruned at read time
//!   against the system clock.
//!
//! Locking: one `std::sync::Mutex` around all state. No `.await` ever
//! happens while it is held, so holding a blocking mutex inside async
//! methods is safe and keeps the implementation dependency-free.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::future::BoxFuture;
use mush_core::{Obj, ObjId};

use crate::store::ObjectStore;
use crate::types::{
    AccountRec, BanRec, CharacterRef, ModuleWrite, StoreError, TokenRecord, WorldSnapshot,
    WriteBatch, WriteReceipt, validate_module_key, validate_module_ns, validate_module_op,
    validate_token_hash,
};

/// See the [module docs](self).
#[derive(Debug, Default)]
pub struct InMemoryStore {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    /// id → serialized `Obj` JSON doc (what Redis holds under `obj:{id}`).
    objects: BTreeMap<u32, String>,
    /// `meta:next_id`.
    next_id: Option<u32>,
    /// `meta:schema_version`.
    schema_version: Option<u32>,
    /// lowercase name → serialized `AccountRec` JSON doc.
    accounts: HashMap<String, String>,
    /// ip → serialized `BanRec` JSON doc.
    bans: HashMap<String, String>,
    /// token hash → serialized `TokenRecord` JSON doc (`token:{hash}` in
    /// Redis). Expired entries are pruned lazily at read time against the
    /// system clock, mirroring Redis TTL — like `bans`.
    tokens: HashMap<String, String>,
    /// `(ns, key)` → serialized module JSON doc (`mod:{ns}:{key}` in
    /// Redis). The tuple key makes namespace isolation structural — no
    /// prefix matching to get wrong.
    module_kv: BTreeMap<(String, String), String>,
    /// `(ns, key)` → capped module stream (`mod:{ns}:{key}` in Redis).
    module_streams: BTreeMap<(String, String), MemStream>,
    /// Highest acknowledged batch serial (debug/ops visibility).
    last_serial: Option<u64>,
    closed: bool,
}

/// One in-memory module stream: a capped `VecDeque` mirroring
/// `XADD ... MAXLEN ~ n`. Trimming is **exact** here (a valid instance of
/// Redis's "at least `maxlen`, approximately" contract, which may retain
/// more). Ids are `"{seq}-0"` with `seq` starting at 1 and monotonically
/// increasing for the stream's lifetime — trims never reset it (like Redis,
/// a `Del` destroys the stream, so a recreated stream starts fresh).
#[derive(Debug, Default)]
struct MemStream {
    /// Sequence number the next appended entry receives.
    next_seq: u64,
    /// `(seq, serialized value JSON)`, oldest first.
    entries: VecDeque<(u64, String)>,
}

/// A module op validated and serialized ahead of the state lock, so a bad
/// op fails the whole batch before anything mutates.
enum PlannedModuleOp {
    JsonSet {
        ns: String,
        key: String,
        doc: String,
    },
    Del {
        ns: String,
        key: String,
    },
    StreamAppend {
        ns: String,
        key: String,
        doc: String,
        maxlen: u32,
    },
}

impl InMemoryStore {
    /// A fresh, empty store ([`ObjectStore::load_world`] returns `None`).
    #[must_use]
    pub fn new() -> InMemoryStore {
        InMemoryStore::default()
    }

    /// Lock the state, failing if the store is closed.
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Inner>, StoreError> {
        let inner = self.inner.lock().expect("in-memory store lock poisoned");
        if inner.closed {
            return Err(StoreError::Closed);
        }
        Ok(inner)
    }
}

/// Unix seconds from the system clock (the store crate may read wall time;
/// `mush-core` never does).
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

impl ObjectStore for InMemoryStore {
    fn load_world(&self) -> BoxFuture<'_, Result<Option<WorldSnapshot>, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            if inner.objects.is_empty() && inner.next_id.is_none() {
                return Ok(None);
            }
            let Some(next_id) = inner.next_id else {
                return Err(StoreError::Corrupt(
                    "objects present but next_id missing".into(),
                ));
            };
            let mut objects = Vec::with_capacity(inner.objects.len());
            let mut corrupt = 0usize;
            for (id, doc) in &inner.objects {
                match serde_json::from_str::<Obj>(doc) {
                    Ok(obj) => objects.push(obj),
                    Err(e) => {
                        corrupt += 1;
                        tracing::error!(id, error = %e, "skipping corrupt object doc");
                    }
                }
            }
            if corrupt > 0 {
                tracing::error!(corrupt, "corrupt object docs skipped during load");
            }
            let schema_version = inner.schema_version.unwrap_or_else(|| {
                tracing::warn!("non-empty store without schema_version; assuming v1");
                1
            });
            Ok(Some(WorldSnapshot {
                objects,
                next_id,
                schema_version,
                corrupt_docs: corrupt,
            }))
        })
    }

    fn write(&self, batch: WriteBatch) -> BoxFuture<'_, Result<WriteReceipt, StoreError>> {
        Box::pin(async move {
            let serial = batch.serial;
            if batch.is_empty() {
                return Ok(WriteReceipt { serial });
            }
            // Validate + serialize (the only fallible steps) before touching
            // state, so a failure leaves the store untouched: all-or-nothing.
            let mut docs = Vec::with_capacity(batch.upserts.len());
            for obj in &batch.upserts {
                docs.push((obj.id.0, serde_json::to_string(obj)?));
            }
            let mut planned = Vec::with_capacity(batch.module_ops.len());
            for op in &batch.module_ops {
                validate_module_op(op)?;
                let ns = op.ns.to_string();
                planned.push(match &op.op {
                    ModuleWrite::JsonSet { key, value } => PlannedModuleOp::JsonSet {
                        ns,
                        key: key.to_string(),
                        doc: serde_json::to_string(value)?,
                    },
                    ModuleWrite::Del { key } => PlannedModuleOp::Del {
                        ns,
                        key: key.to_string(),
                    },
                    ModuleWrite::StreamAppend { key, value, maxlen } => {
                        PlannedModuleOp::StreamAppend {
                            ns,
                            key: key.to_string(),
                            doc: serde_json::to_string(value)?,
                            maxlen: *maxlen,
                        }
                    }
                });
            }
            let mut inner = self.lock()?;
            for (id, doc) in docs {
                inner.objects.insert(id, doc);
            }
            for id in &batch.deletes {
                inner.objects.remove(&id.0);
            }
            // Module ops after the object writes, in queue order.
            for op in planned {
                match op {
                    PlannedModuleOp::JsonSet { ns, key, doc } => {
                        inner.module_kv.insert((ns, key), doc);
                    }
                    PlannedModuleOp::Del { ns, key } => {
                        // Redis DEL is type-agnostic: it removes whatever
                        // lives at mod:{ns}:{key}, JSON doc or stream.
                        inner.module_kv.remove(&(ns.clone(), key.clone()));
                        inner.module_streams.remove(&(ns, key));
                    }
                    PlannedModuleOp::StreamAppend {
                        ns,
                        key,
                        doc,
                        maxlen,
                    } => {
                        let stream = inner.module_streams.entry((ns, key)).or_default();
                        stream.next_seq += 1;
                        let seq = stream.next_seq;
                        stream.entries.push_back((seq, doc));
                        while stream.entries.len() > maxlen as usize {
                            stream.entries.pop_front();
                        }
                    }
                }
            }
            if let Some(next_id) = batch.next_id {
                inner.next_id = Some(next_id);
            }
            inner.last_serial = Some(serial);
            Ok(WriteReceipt { serial })
        })
    }

    fn create_account(&self, rec: AccountRec) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            if rec.name.is_empty() {
                return Err(StoreError::Invalid("empty account name".into()));
            }
            let key = rec.name.to_lowercase();
            let doc = serde_json::to_string(&rec)?;
            let mut inner = self.lock()?;
            if inner.accounts.contains_key(&key) {
                return Err(StoreError::AccountExists);
            }
            inner.accounts.insert(key, doc);
            Ok(())
        })
    }

    fn get_account<'a>(
        &'a self,
        name: &'a str,
    ) -> BoxFuture<'a, Result<Option<AccountRec>, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            match inner.accounts.get(&name.to_lowercase()) {
                Some(doc) => Ok(Some(serde_json::from_str(doc)?)),
                None => Ok(None),
            }
        })
    }

    fn link_character<'a>(
        &'a self,
        account: &'a str,
        id: ObjId,
        char_name: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let key = account.to_lowercase();
            let mut inner = self.lock()?;
            let Some(doc) = inner.accounts.get(&key) else {
                return Err(StoreError::NoSuchAccount(account.to_string()));
            };
            let mut rec: AccountRec = serde_json::from_str(doc)?;
            rec.characters.push(CharacterRef {
                id,
                name: char_name.to_string(),
            });
            let doc = serde_json::to_string(&rec)?;
            inner.accounts.insert(key, doc);
            Ok(())
        })
    }

    fn list_accounts(&self) -> BoxFuture<'_, Result<Vec<AccountRec>, StoreError>> {
        Box::pin(async move {
            let inner = self.lock()?;
            let mut out = Vec::with_capacity(inner.accounts.len());
            for doc in inner.accounts.values() {
                match serde_json::from_str::<AccountRec>(doc) {
                    Ok(rec) => out.push(rec),
                    Err(e) => tracing::error!(error = %e, "skipping corrupt account doc"),
                }
            }
            Ok(out)
        })
    }

    fn set_account_password<'a>(
        &'a self,
        account: &'a str,
        argon2_hash: &'a str,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            let key = account.to_lowercase();
            let mut inner = self.lock()?;
            let Some(doc) = inner.accounts.get(&key) else {
                return Ok(false);
            };
            let mut rec: AccountRec = serde_json::from_str(doc)?;
            rec.argon2_hash = argon2_hash.to_string();
            let doc = serde_json::to_string(&rec)?;
            inner.accounts.insert(key, doc);
            Ok(true)
        })
    }

    fn set_ban<'a>(
        &'a self,
        ip: &'a str,
        until_unix: Option<i64>,
        reason: &'a str,
    ) -> BoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            if ip.is_empty() {
                return Err(StoreError::Invalid("empty ban ip".into()));
            }
            let rec = BanRec {
                ip: ip.to_string(),
                until_unix,
                reason: reason.to_string(),
            };
            let doc = serde_json::to_string(&rec)?;
            let mut inner = self.lock()?;
            if let Some(until) = until_unix
                && until <= now_unix()
            {
                // Mirrors Redis: a key set with EXAT in the past is
                // immediately gone.
                inner.bans.remove(ip);
                return Ok(());
            }
            inner.bans.insert(ip.to_string(), doc);
            Ok(())
        })
    }

    fn get_bans(&self) -> BoxFuture<'_, Result<Vec<BanRec>, StoreError>> {
        Box::pin(async move {
            let now = now_unix();
            let mut inner = self.lock()?;
            let mut out = Vec::new();
            let mut expired = Vec::new();
            for (ip, doc) in &inner.bans {
                match serde_json::from_str::<BanRec>(doc) {
                    Ok(rec) => {
                        if rec.until_unix.is_some_and(|until| until <= now) {
                            expired.push(ip.clone());
                        } else {
                            out.push(rec);
                        }
                    }
                    Err(e) => tracing::error!(ip, error = %e, "skipping corrupt ban doc"),
                }
            }
            // Prune, as Redis TTL would have.
            for ip in expired {
                inner.bans.remove(&ip);
            }
            Ok(out)
        })
    }

    fn clear_ban<'a>(&'a self, ip: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            let mut inner = self.lock()?;
            Ok(inner.bans.remove(ip).is_some())
        })
    }

    fn token_put(&self, hash: String, rec: TokenRecord) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            validate_token_hash(&hash)?;
            let doc = serde_json::to_string(&rec)?;
            let mut inner = self.lock()?;
            if rec.expires <= now_unix() {
                // Mirrors Redis: a key set with EXAT in the past is
                // immediately gone.
                inner.tokens.remove(&hash);
                return Ok(());
            }
            inner.tokens.insert(hash, doc);
            Ok(())
        })
    }

    fn token_get<'a>(
        &'a self,
        hash: &'a str,
    ) -> BoxFuture<'a, Result<Option<TokenRecord>, StoreError>> {
        Box::pin(async move {
            validate_token_hash(hash)?;
            let mut inner = self.lock()?;
            let Some(doc) = inner.tokens.get(hash) else {
                return Ok(None);
            };
            let rec: TokenRecord = serde_json::from_str(doc)?;
            if rec.expires <= now_unix() {
                // Lazy delete, as Redis TTL would have.
                inner.tokens.remove(hash);
                return Ok(None);
            }
            Ok(Some(rec))
        })
    }

    fn token_touch<'a>(
        &'a self,
        hash: &'a str,
        new_expires: i64,
    ) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            validate_token_hash(hash)?;
            let mut inner = self.lock()?;
            let Some(doc) = inner.tokens.get(hash) else {
                return Ok(false);
            };
            let mut rec: TokenRecord = serde_json::from_str(doc)?;
            if rec.expires <= now_unix() {
                // Expired = missing; a touch never resurrects.
                inner.tokens.remove(hash);
                return Ok(false);
            }
            if new_expires <= now_unix() {
                // Sliding into the past is a delete.
                inner.tokens.remove(hash);
                return Ok(true);
            }
            rec.expires = new_expires;
            let doc = serde_json::to_string(&rec)?;
            inner.tokens.insert(hash.to_string(), doc);
            Ok(true)
        })
    }

    fn token_del<'a>(&'a self, hash: &'a str) -> BoxFuture<'a, Result<bool, StoreError>> {
        Box::pin(async move {
            validate_token_hash(hash)?;
            let mut inner = self.lock()?;
            let Some(doc) = inner.tokens.remove(hash) else {
                return Ok(false);
            };
            // An expired straggler is already "gone": removed, not reported.
            let live =
                serde_json::from_str::<TokenRecord>(&doc).is_ok_and(|rec| rec.expires > now_unix());
            Ok(live)
        })
    }

    fn tokens_del_account<'a>(
        &'a self,
        account: &'a str,
    ) -> BoxFuture<'a, Result<u64, StoreError>> {
        Box::pin(async move {
            let account_lc = account.to_lowercase();
            let now = now_unix();
            let mut inner = self.lock()?;
            let mut doomed = Vec::new();
            let mut live = 0u64;
            for (hash, doc) in &inner.tokens {
                match serde_json::from_str::<TokenRecord>(doc) {
                    Ok(rec) => {
                        let expired = rec.expires <= now;
                        if expired {
                            // Prune, as Redis TTL would have — not counted.
                            doomed.push(hash.clone());
                        } else if rec.account.to_lowercase() == account_lc {
                            doomed.push(hash.clone());
                            live += 1;
                        }
                    }
                    Err(e) => tracing::error!(error = %e, "skipping corrupt token doc"),
                }
            }
            for hash in doomed {
                inner.tokens.remove(&hash);
            }
            Ok(live)
        })
    }

    fn module_json_get<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
    ) -> BoxFuture<'a, Result<Option<serde_json::Value>, StoreError>> {
        Box::pin(async move {
            validate_module_ns(ns)?;
            validate_module_key(key)?;
            let inner = self.lock()?;
            match inner.module_kv.get(&(ns.to_string(), key.to_string())) {
                Some(doc) => Ok(Some(serde_json::from_str(doc)?)),
                None => Ok(None),
            }
        })
    }

    fn module_json_list<'a>(
        &'a self,
        ns: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        Box::pin(async move {
            validate_module_ns(ns)?;
            let inner = self.lock()?;
            let mut out = Vec::new();
            for ((doc_ns, key), doc) in &inner.module_kv {
                if doc_ns != ns {
                    continue;
                }
                match serde_json::from_str(doc) {
                    Ok(value) => out.push((key.clone(), value)),
                    Err(e) => {
                        tracing::error!(ns, key, error = %e, "skipping corrupt module doc");
                    }
                }
            }
            Ok(out)
        })
    }

    fn stream_range<'a>(
        &'a self,
        ns: &'a str,
        key: &'a str,
        count_latest: u32,
    ) -> BoxFuture<'a, Result<Vec<(String, serde_json::Value)>, StoreError>> {
        Box::pin(async move {
            validate_module_ns(ns)?;
            validate_module_key(key)?;
            if count_latest == 0 {
                return Ok(Vec::new());
            }
            let inner = self.lock()?;
            let Some(stream) = inner.module_streams.get(&(ns.to_string(), key.to_string())) else {
                return Ok(Vec::new());
            };
            let skip = stream.entries.len().saturating_sub(count_latest as usize);
            let mut out = Vec::with_capacity(stream.entries.len() - skip);
            for (seq, doc) in stream.entries.iter().skip(skip) {
                out.push((format!("{seq}-0"), serde_json::from_str(doc)?));
            }
            Ok(out)
        })
    }

    fn get_schema_version(&self) -> BoxFuture<'_, Result<Option<u32>, StoreError>> {
        Box::pin(async move { Ok(self.lock()?.schema_version) })
    }

    fn set_schema_version(&self, version: u32) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            self.lock()?.schema_version = Some(version);
            Ok(())
        })
    }

    fn bgsave(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            drop(self.lock()?);
            Ok(()) // nothing to snapshot; the RDB story is Redis's
        })
    }

    fn drain(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            drop(self.lock()?);
            Ok(()) // writes are acknowledged synchronously; nothing pends
        })
    }

    fn close(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            self.inner
                .lock()
                .expect("in-memory store lock poisoned")
                .closed = true;
            Ok(())
        })
    }
}

/// Memory-implementation-specific stream semantics (the cross-impl behavior
/// lives in [`crate::suite`]): exact trimming to `maxlen`, `"N-0"` id
/// format, monotonic ids across trims, and type-agnostic `Del`.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{ModuleOp, WriteBatch};

    fn append(ns: &str, key: &str, value: serde_json::Value, maxlen: u32) -> ModuleOp {
        ModuleOp {
            ns: ns.into(),
            op: ModuleWrite::StreamAppend {
                key: key.into(),
                value,
                maxlen,
            },
        }
    }

    async fn write_ops(store: &InMemoryStore, serial: u64, module_ops: Vec<ModuleOp>) {
        store
            .write(WriteBatch {
                serial,
                module_ops,
                ..WriteBatch::default()
            })
            .await
            .expect("module-op batch");
    }

    #[tokio::test]
    async fn stream_ids_are_n_dash_zero_and_survive_trims() {
        let store = InMemoryStore::new();
        // Five appends, cap 2: exact trim keeps exactly the newest two, and
        // the surviving ids show the counter never reset.
        for i in 1..=5 {
            write_ops(&store, i, vec![append("m", "log", serde_json::json!(i), 2)]).await;
        }
        let entries = store.stream_range("m", "log", 10).await.expect("range");
        let got: Vec<(&str, i64)> = entries
            .iter()
            .map(|(id, v)| (id.as_str(), v.as_i64().unwrap()))
            .collect();
        assert_eq!(got, [("4-0", 4), ("5-0", 5)], "exact trim, monotonic ids");

        // The next append continues the sequence.
        write_ops(&store, 6, vec![append("m", "log", serde_json::json!(6), 2)]).await;
        let entries = store.stream_range("m", "log", 1).await.expect("range");
        assert_eq!(entries[0].0, "6-0");
    }

    #[tokio::test]
    async fn del_removes_streams_too() {
        let store = InMemoryStore::new();
        write_ops(
            &store,
            1,
            vec![
                append("m", "log", serde_json::json!("entry"), 8),
                ModuleOp {
                    ns: "m".into(),
                    op: ModuleWrite::JsonSet {
                        key: "doc".into(),
                        value: serde_json::json!({"a": 1}),
                    },
                },
            ],
        )
        .await;
        write_ops(
            &store,
            2,
            vec![
                ModuleOp {
                    ns: "m".into(),
                    op: ModuleWrite::Del { key: "log".into() },
                },
                ModuleOp {
                    ns: "m".into(),
                    op: ModuleWrite::Del { key: "doc".into() },
                },
            ],
        )
        .await;
        assert!(
            store
                .stream_range("m", "log", 10)
                .await
                .expect("range")
                .is_empty(),
            "Del must destroy the stream (Redis DEL is type-agnostic)"
        );
        assert_eq!(
            store.module_json_get("m", "doc").await.expect("get"),
            None,
            "Del must remove the JSON doc"
        );
        // A recreated stream is a *new* stream: the counter starts over.
        write_ops(
            &store,
            3,
            vec![append("m", "log", serde_json::json!("x"), 8)],
        )
        .await;
        let entries = store.stream_range("m", "log", 10).await.expect("range");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].0, "1-0");
    }
}

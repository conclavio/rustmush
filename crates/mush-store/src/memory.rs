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
//! - ban expiry mirrors Redis TTL: expired entries are pruned at read time
//!   against the system clock.
//!
//! Locking: one `std::sync::Mutex` around all state. No `.await` ever
//! happens while it is held, so holding a blocking mutex inside async
//! methods is safe and keeps the implementation dependency-free.

use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::future::BoxFuture;
use mush_core::{Obj, ObjId};

use crate::store::ObjectStore;
use crate::types::{
    AccountRec, BanRec, CharacterRef, StoreError, WorldSnapshot, WriteBatch, WriteReceipt,
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
    /// Highest acknowledged batch serial (debug/ops visibility).
    last_serial: Option<u64>,
    closed: bool,
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
            // Serialize (the only fallible step) before touching state, so a
            // failure leaves the store untouched: all-or-nothing.
            let mut docs = Vec::with_capacity(batch.upserts.len());
            for obj in &batch.upserts {
                docs.push((obj.id.0, serde_json::to_string(obj)?));
            }
            let mut inner = self.lock()?;
            for (id, doc) in docs {
                inner.objects.insert(id, doc);
            }
            for id in &batch.deletes {
                inner.objects.remove(&id.0);
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

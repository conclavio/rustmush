//! Boot: rebuild the [`EconomyStore`] by REPLAYING THE JOURNAL (E2b).
//!
//! The journal is the sole authority (doc 08 §7.5). Every derived structure — the
//! account balances, the idempotency index, the reversed set, the txid index, and
//! the allocators — is re-derived here from the full journal in `data.kv`, so a
//! companion projection/idempotency drop can never cause a double-post (R-B2).
//!
//! Reconciliation is DIRECTIONAL (R-B1): a stored account projection that merely
//! LAGS the journal (a dropped companion write — the routine case the retained-seal
//! table exists to heal) is silently re-derived to the journal head; only a stored
//! balance that DISAGREES with the journal at its own claimed sequence (drift), a
//! projection AHEAD of the journal (a lost tail), an unreplayable journal (a
//! contiguity breach or an unknown referenced account), or a per-instrument
//! conservation break (R-R5) latches a freeze / fails the boot. The safe failure is
//! always *no* spending authority.

use std::collections::BTreeMap;

use mush_core::module::{ModError, ModuleBootData, ModuleCtx};
use mush_core::{Ledger, LedgerChunk, scan_contiguity};

use crate::amount::Amount;
use crate::api::{self, META_KEY};
use crate::ids::{AccountId, InstrumentId, TransactionId};
use crate::module::EconomyStore;
use crate::record::{Account, EconomicTransaction, IdempotencyRecord, InstrumentDef};
use crate::snapshot::EconomySnapshot;

/// Rebuild the [`EconomyStore`] from boot data.
pub(crate) fn load_boot(ctx: &mut ModuleCtx<'_>, data: ModuleBootData) -> Result<EconomyStore, ModError> {
    // Unreplayable journal → fail the boot (never destructively "recover" money).
    if !scan_contiguity(&data.kv).is_empty() {
        return Err(ModError::msg("economy: journal contiguity breach; refusing to boot on a hole"));
    }
    let mut journal = Ledger::new();
    journal.restore_from_kv(&data.kv);

    // Parse the persisted docs. Instruments + accounts are containers (a corrupt one
    // fails the boot — proceeding would classify live money as orphaned).
    let mut instruments: BTreeMap<InstrumentId, InstrumentDef> = BTreeMap::new();
    let mut stored_accounts: BTreeMap<AccountId, Account> = BTreeMap::new();
    let mut chunks: Vec<LedgerChunk> = Vec::new();
    let mut meta: Option<serde_json::Value> = None;

    for (key, value) in &data.kv {
        if key == META_KEY {
            meta = Some(value.clone());
        } else if key.starts_with("log:journal:") {
            let chunk = serde_json::from_value::<LedgerChunk>(value.clone())
                .map_err(|e| ModError::msg(format!("economy: bad journal chunk {key:?}: {e}")))?;
            chunks.push(chunk);
        } else if let Some(id) = key.strip_prefix("instrument:") {
            let def = serde_json::from_value::<InstrumentDef>(value.clone())
                .map_err(|e| ModError::msg(format!("economy: bad instrument doc {key:?}: {e}")))?;
            if def.id.as_str() != id {
                return Err(ModError::msg(format!("economy: instrument doc {key:?} claims id {}", def.id)));
            }
            instruments.insert(def.id.clone(), def);
        } else if let Some(id) = key.strip_prefix("account:") {
            let acct = serde_json::from_value::<Account>(value.clone())
                .map_err(|e| ModError::msg(format!("economy: bad account doc {key:?}: {e}")))?;
            let id = id.parse::<u64>().map_err(|_| ModError::msg(format!("economy: bad account key {key:?}")))?;
            if acct.id != AccountId(id) {
                return Err(ModError::msg(format!("economy: account doc {key:?} claims id {}", acct.id)));
            }
            stored_accounts.insert(acct.id, acct);
        } else {
            tracing::warn!(%key, "economy: unknown persisted key ignored");
        }
    }

    // Order the full journal by sequence (chunks may arrive unordered).
    let mut entries: Vec<(u64, EconomicTransaction)> = Vec::new();
    for chunk in chunks {
        for e in chunk.entries {
            let tx = serde_json::from_value::<EconomicTransaction>(e.value)
                .map_err(|err| ModError::msg(format!("economy: bad journal entry seq {}: {err}", e.seq)))?;
            entries.push((e.seq, tx));
        }
    }
    entries.sort_by_key(|(seq, _)| *seq);
    let journal_max: Option<u64> = entries.last().map(|(seq, _)| *seq);

    // Replay: fold deltas into a running per-account balance; derive the idempotency
    // index, reversed set, and txid index; capture each account's balance at its own
    // stored `last_journal_seq` (the drift detector).
    let mut running: BTreeMap<AccountId, i128> = BTreeMap::new();
    let mut last_touch: BTreeMap<AccountId, u64> = BTreeMap::new();
    let mut snapshot_at_stored: BTreeMap<AccountId, i128> = BTreeMap::new();
    let mut transactions: BTreeMap<TransactionId, EconomicTransaction> = BTreeMap::new();
    let mut snapshot = EconomySnapshot::new();
    for def in instruments.values() {
        snapshot.put_instrument(def.clone());
    }
    let mut max_txid: u64 = 0;

    for (seq, tx) in &entries {
        for line in &tx.lines {
            // A journal line referencing an account with no doc is corruption — the
            // account metadata was lost while its money history survived. Fail closed.
            if !stored_accounts.contains_key(&line.account) {
                return Err(ModError::msg(format!(
                    "economy: journal seq {seq} references unknown account {}; refusing to boot",
                    line.account
                )));
            }
            *running.entry(line.account).or_insert(0) += line.delta.widen();
            last_touch.insert(line.account, *seq);
            if stored_accounts.get(&line.account).map(|a| a.last_journal_seq) == Some(*seq) {
                snapshot_at_stored.insert(line.account, *running.get(&line.account).expect("just inserted"));
            }
        }
        // Idempotency index (journal-derived — the double-post backstop).
        snapshot.put_idempotency(IdempotencyRecord {
            idempotency: tx.idempotency.clone(),
            plan_hash: tx.plan_hash,
            transaction: tx.id,
            posted_at: tx.posted_at,
        });
        if let Some(target) = tx.reverses {
            snapshot.mark_reversed(target);
        }
        max_txid = max_txid.max(tx.id.0);
        transactions.insert(tx.id, tx.clone());
    }

    // Directional self-heal + drift/ahead detection per account.
    let mut frozen = false;
    let mut healed: Vec<Account> = Vec::new();
    for (id, mut acct) in stored_accounts {
        let derived = i64::try_from(*running.get(&id).unwrap_or(&0)).ok().map(Amount::from_minor);
        let Some(derived) = derived else {
            tracing::error!(account = id.0, "economy: replayed balance does not fit i64; freezing");
            frozen = true;
            acct.status = crate::record::AccountStatus::Frozen;
            snapshot.put_account(acct);
            continue;
        };
        // AHEAD: the projection remembers a journal entry the journal no longer has.
        if acct.last_journal_seq > journal_max.unwrap_or(0) && acct.last_journal_seq != 0 {
            tracing::error!(account = id.0, stored_seq = acct.last_journal_seq, journal_max = ?journal_max, "economy: projection ahead of the journal (lost tail); freezing");
            frozen = true;
        }
        // DRIFT: the stored balance disagrees with the journal at its own claimed seq.
        let claimed = snapshot_at_stored.get(&id).copied().unwrap_or(0);
        if Amount::from_widened(claimed) != Some(acct.posted_balance) {
            tracing::error!(account = id.0, "economy: stored balance drifts from the journal at its claimed seq; freezing");
            frozen = true;
        }
        // Adopt the journal-derived balance (heals a benign lag; a no-op when clean).
        if acct.posted_balance != derived || acct.last_journal_seq != last_touch.get(&id).copied().unwrap_or(0) {
            acct.posted_balance = derived;
            acct.last_journal_seq = last_touch.get(&id).copied().unwrap_or(0);
            healed.push(acct.clone());
        }
        snapshot.put_account(acct);
    }

    // Conservation tripwire (R-R5): with a zero start and all mint/burn through a
    // system source/sink, every instrument's balances sum to exactly zero. A
    // non-zero sum is unambiguous corruption → freeze the whole economy.
    let mut per_instrument: BTreeMap<InstrumentId, i128> = BTreeMap::new();
    for a in snapshot.accounts() {
        *per_instrument.entry(a.instrument.clone()).or_insert(0) += a.posted_balance.widen();
    }
    for (instrument, sum) in &per_instrument {
        if *sum != 0 {
            tracing::error!(%instrument, sum, "economy: per-instrument conservation broken; freezing the economy");
            frozen = true;
        }
    }

    // If frozen, freeze every account too (belt-and-suspenders: no posting path).
    if frozen {
        let ids: Vec<AccountId> = snapshot.accounts().map(|a| a.id).collect();
        for id in ids {
            if let Some(a) = snapshot.account_mut(id) {
                a.status = crate::record::AccountStatus::Frozen;
            }
        }
    }

    // Allocators: meta is authoritative but floored at one past the journal-derived
    // max so a stale meta never re-mints a live id.
    let next_account = meta_u64(&meta, "next_account_id").max(next_after(snapshot.accounts().map(|a| a.id.0).max()));
    let next_transaction = meta_u64(&meta, "next_transaction_id").max(if entries.is_empty() { 0 } else { max_txid + 1 });

    // Self-heal writes (only balances/markers that actually changed, unless frozen —
    // then persist the frozen status of everything).
    if frozen {
        let docs: Vec<Account> = snapshot.accounts().cloned().collect();
        for a in &docs {
            api::persist_account(ctx.out, a);
        }
    } else {
        for a in &healed {
            api::persist_account(ctx.out, a);
        }
    }

    Ok(EconomyStore::from_parts(snapshot, journal, transactions, next_account, next_transaction, frozen))
}

fn meta_u64(meta: &Option<serde_json::Value>, field: &str) -> u64 {
    meta.as_ref().and_then(|m| m.get(field)).and_then(serde_json::Value::as_u64).unwrap_or(0)
}
fn next_after(max: Option<u64>) -> u64 {
    max.map_or(0, |m| m + 1)
}

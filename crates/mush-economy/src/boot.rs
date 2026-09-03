//! Boot: rebuild the [`EconomyStore`] by REPLAYING THE JOURNAL (E2b, extended in
//! E3b1).
//!
//! The journal is the sole authority (doc 08 §7.5). Every derived structure — the
//! account balances, the **asset title projections**, the idempotency index, the
//! reversed set, the txid index, and the allocators — is re-derived here from the
//! full journal in `data.kv`, so a companion projection/idempotency drop can never
//! cause a double-post (R-B2) or lose a paid-for title (D-E3-1).
//!
//! Reconciliation is DIRECTIONAL (R-B1): a stored projection that merely LAGS the
//! journal (a dropped companion write — the routine case the retained-seal table
//! exists to heal) is silently re-derived to the journal head; only a stored value
//! that DISAGREES with the journal at its own claimed sequence (drift), a projection
//! AHEAD of the journal (a lost tail), an unreplayable journal (a contiguity breach
//! or an unknown referenced account), or a per-instrument conservation break (R-R5)
//! latches a freeze / fails the boot. The safe failure is always *no* spending
//! authority.
//!
//! ## E3b1's four extra passes
//!
//! 1. **Asset projections.** `asset:{obj}` is a projection over the journal-authority
//!    fields; the `frozen` latch is doc authority and is NEVER compared (C1/C2 — a
//!    latch that tripped drift could never be cleared, and one carried in an effect
//!    could never be set). A missing doc for a journaled asset rebuilds **frozen**
//!    ("doc-lost"); a doc with no journal history is **not loaded** (an unjournaled
//!    doc is not a title — C6).
//! 2. **Vaults** are containers: corrupt → fail boot; a backing that is gone,
//!    `GOING`, or reported invalid → `Frozen` + re-persist, never deleted.
//! 3. **Liveness.** A titled object, title owner, assignee, creditor or claimant
//!    that is missing or `GOING` freezes the asset — the boot half of the
//!    destruction backstop (only `@destroy` emits `ObjDestroyed`).
//! 4. **Custody reconciliation.** `custody_effects` replay to a last-known-vault map
//!    and the resolved provider is asked whether the vault still holds each
//!    deposited object. `Some(false)` on a titled object freezes it
//!    (`custody-mismatch`); **`None` — the provider cannot answer, e.g. its own
//!    state has not booted yet — is a warning and never a freeze.** Custody effects
//!    are audit-only: nothing here derives a location from them.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::module::{ModError, ModuleBootData, ModuleCtx};
use mush_core::{Flags, Ledger, LedgerChunk, ObjId, World, scan_contiguity};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::api::{self, META_KEY};
use crate::custody::CustodyTable;
use crate::ids::{AccountId, InstrumentId, TransactionId, VaultId};
use crate::module::EconomyStore;
use crate::record::{
    Account, CustodyDirection, EconomicAsset, EconomicTransaction, Encumbrance, IdempotencyRecord,
    InstrumentDef, Vault, VaultStatus,
};
use crate::snapshot::EconomySnapshot;

/// Rebuild the [`EconomyStore`] from boot data.
#[allow(clippy::too_many_lines)]
pub(crate) fn load_boot(
    ctx: &mut ModuleCtx<'_>,
    data: ModuleBootData,
    providers: &CustodyTable,
) -> Result<EconomyStore, ModError> {
    // Unreplayable journal → fail the boot (never destructively "recover" money).
    if !scan_contiguity(&data.kv).is_empty() {
        return Err(ModError::msg("economy: journal contiguity breach; refusing to boot on a hole"));
    }
    let mut journal = Ledger::new();
    journal.restore_from_kv(&data.kv);

    // Parse the persisted docs. Instruments + accounts + vaults are containers (a
    // corrupt one fails the boot — proceeding would classify live money, or a live
    // custody binding, as absent). An asset doc is a projection, but a corrupt one
    // still fails: its `frozen` latch is doc authority and unreadable, and silently
    // un-freezing a recovering title is exactly the wrong direction.
    let mut instruments: BTreeMap<InstrumentId, InstrumentDef> = BTreeMap::new();
    let mut stored_accounts: BTreeMap<AccountId, Account> = BTreeMap::new();
    let mut stored_assets: BTreeMap<ObjId, EconomicAsset> = BTreeMap::new();
    let mut stored_vaults: BTreeMap<VaultId, Vault> = BTreeMap::new();
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
        } else if let Some(id) = key.strip_prefix("asset:") {
            let asset = serde_json::from_value::<EconomicAsset>(value.clone())
                .map_err(|e| ModError::msg(format!("economy: bad asset doc {key:?}: {e}")))?;
            let id = id.parse::<u32>().map_err(|_| ModError::msg(format!("economy: bad asset key {key:?}")))?;
            if asset.object != ObjId(id) {
                return Err(ModError::msg(format!("economy: asset doc {key:?} claims object {}", asset.object)));
            }
            stored_assets.insert(asset.object, asset);
        } else if let Some(id) = key.strip_prefix("vault:") {
            let vault = serde_json::from_value::<Vault>(value.clone())
                .map_err(|e| ModError::msg(format!("economy: bad vault doc {key:?}: {e}")))?;
            let id = id.parse::<u64>().map_err(|_| ModError::msg(format!("economy: bad vault key {key:?}")))?;
            if vault.id != VaultId(id) {
                return Err(ModError::msg(format!("economy: vault doc {key:?} claims id {}", vault.id)));
            }
            stored_vaults.insert(vault.id, vault);
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
    // stored `last_journal_seq` (the drift detector). E3: fold the asset effects the
    // same way, and collect the audit-only custody effects.
    let mut running: BTreeMap<AccountId, i128> = BTreeMap::new();
    let mut last_touch: BTreeMap<AccountId, u64> = BTreeMap::new();
    let mut snapshot_at_stored: BTreeMap<AccountId, i128> = BTreeMap::new();
    let mut journal_assets: BTreeMap<ObjId, EconomicAsset> = BTreeMap::new();
    let mut asset_at_stored: BTreeMap<ObjId, EconomicAsset> = BTreeMap::new();
    let mut last_custody: BTreeMap<ObjId, (VaultId, CustodyDirection)> = BTreeMap::new();
    let mut max_custody_vault: Option<u64> = None;
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
        // E3: the title effects carry the full post-state; the LAST one per object
        // wins. An effect-only entry contributes zero to every balance (invariant 10).
        for effect in &tx.asset_effects {
            let mut post = effect.post.clone();
            post.last_journal_seq = *seq;
            // The doc-authority latch is never replayed (D-E3-1).
            post.frozen = false;
            post.frozen_reason = None;
            if stored_assets.get(&effect.object).map(|a| a.last_journal_seq) == Some(*seq) {
                asset_at_stored.insert(effect.object, post.clone());
            }
            journal_assets.insert(effect.object, post);
        }
        for effect in &tx.custody_effects {
            last_custody.insert(effect.object, (effect.vault, effect.direction));
            max_custody_vault = Some(max_custody_vault.map_or(effect.vault.0, |m: u64| m.max(effect.vault.0)));
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

    // ---- E3: asset projections ------------------------------------------
    let mut healed_assets: Vec<EconomicAsset> = Vec::new();
    for (object, derived) in &journal_assets {
        match stored_assets.remove(object) {
            None => {
                // A journaled title whose doc vanished: rebuild it, but NEVER silently
                // heal the latch — a lost doc is a lost latch, so it comes back frozen.
                tracing::error!(object = object.0, "economy: asset doc lost; rebuilding from the journal, frozen");
                let mut a = derived.clone();
                a.frozen = true;
                a.frozen_reason = Some(SmolStr::new_static("doc-lost"));
                healed_assets.push(a.clone());
                snapshot.put_asset(a);
            }
            Some(doc) => {
                if doc.last_journal_seq > journal_max.unwrap_or(0) {
                    tracing::error!(object = object.0, stored_seq = doc.last_journal_seq, journal_max = ?journal_max, "economy: asset projection ahead of the journal (lost tail); freezing");
                    frozen = true;
                } else {
                    match asset_at_stored.get(object) {
                        // The doc claims a sequence at which this object really had an
                        // effect: its journal-authority fields must match it exactly.
                        Some(at) if journal_fields_eq(&doc, at) => {}
                        Some(_) => {
                            tracing::error!(object = object.0, "economy: asset projection drifts from the journal at its claimed seq; freezing");
                            frozen = true;
                        }
                        // ...or a sequence at which it had none: an impossible doc.
                        None => {
                            tracing::error!(object = object.0, stored_seq = doc.last_journal_seq, "economy: asset projection claims a sequence it never appears at; freezing");
                            frozen = true;
                        }
                    }
                }
                // Adopt the journal head (heals a benign lag), keeping the latch.
                let mut a = derived.clone();
                a.frozen = doc.frozen;
                a.frozen_reason = doc.frozen_reason.clone();
                if !journal_fields_eq(&doc, derived) {
                    healed_assets.push(a.clone());
                }
                snapshot.put_asset(a);
            }
        }
    }
    // Docs with NO journal history are not titles (C6): loading one would let a
    // hand-written doc mint ownership. Loud, and left on disk for staff.
    for object in stored_assets.keys() {
        tracing::warn!(object = object.0, "economy: asset doc has no journal history; NOT loaded");
    }

    // ---- E3: vaults -------------------------------------------------------
    let mut healed_vaults: Vec<Vault> = Vec::new();
    for (_, mut v) in stored_vaults {
        if v.status == VaultStatus::Active && !backing_usable(ctx.w, providers, &v) {
            tracing::warn!(vault = v.id.0, "economy: vault backing is gone or unusable; freezing the vault");
            v.status = VaultStatus::Frozen;
            v.revision += 1;
            healed_vaults.push(v.clone());
        }
        snapshot.put_vault(v);
    }

    // ---- E3: custody reconciliation (audit-only; never derives a location) --
    let deposited: Vec<(ObjId, VaultId)> = last_custody
        .iter()
        .filter(|(_, (_, dir))| *dir == CustodyDirection::Deposit)
        .map(|(o, (v, _))| (*o, *v))
        .collect();
    let mut mismatched: BTreeSet<ObjId> = BTreeSet::new();
    for (object, vault_id) in deposited {
        let Some(vault) = snapshot.vault(vault_id) else {
            tracing::warn!(object = object.0, vault = vault_id.0, "economy: custody effect names a vault with no doc");
            continue;
        };
        let answer = crate::custody::resolve(providers, ctx.w, &vault.backing, None)
            .ok()
            .and_then(|p| p.holds(ctx.w, &vault.backing, object));
        match answer {
            Some(true) => {}
            Some(false) => {
                tracing::warn!(object = object.0, vault = vault_id.0, "economy: deposited object is no longer in its vault");
                if snapshot.asset(object).is_some() {
                    mismatched.insert(object);
                }
            }
            // The provider cannot answer (its own state boots later — economy boots
            // before sim). Unknown is NOT drift.
            None => {
                tracing::warn!(object = object.0, vault = vault_id.0, "economy: no custody provider could answer for this vault; skipping reconciliation");
            }
        }
    }
    for object in mismatched {
        if let Some(a) = snapshot.asset_mut(object)
            && !a.frozen
        {
            // `revision` is journal authority and stays put (see `set_asset_frozen`).
            a.frozen = true;
            a.frozen_reason = Some(SmolStr::new_static("custody-mismatch"));
            healed_assets.push(a.clone());
        }
    }

    // ---- E3: liveness (the boot half of the destruction backstop) ---------
    let dead: Vec<ObjId> = snapshot
        .assets()
        .filter(|a| !a.frozen && !asset_is_live(ctx.w, a))
        .map(|a| a.object)
        .collect();
    for object in dead {
        if let Some(a) = snapshot.asset_mut(object) {
            a.frozen = true;
            a.frozen_reason = Some(SmolStr::new_static("principal-gone"));
            healed_assets.push(a.clone());
        }
    }

    // Conservation tripwire (R-R5): with a zero start and all mint/burn through a
    // system source/sink, every instrument's balances sum to exactly zero. A
    // non-zero sum is unambiguous corruption → freeze the whole economy. (An
    // effect-only entry contributes nothing here — it moved no money.)
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
    // Asset latches are per-asset and deliberately untouched: the whole-economy
    // freeze already refuses every asset op, and clobbering the latches would erase
    // which titles actually need staff attention.
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
    let next_vault = meta_u64(&meta, "next_vault_id")
        .max(next_after(max_custody_vault))
        .max(next_after(snapshot.vaults().map(|v| v.id.0).max()));

    // Self-heal writes (only what actually changed, unless frozen — then persist the
    // frozen status of everything).
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
    for a in &healed_assets {
        api::persist_asset(ctx.out, a);
    }
    for v in &healed_vaults {
        api::persist_vault(ctx.out, v);
    }

    Ok(EconomyStore::from_parts(
        snapshot,
        journal,
        transactions,
        next_account,
        next_transaction,
        next_vault,
        frozen,
    ))
}

/// Compare only the **journal-authority** fields of two asset records: the
/// doc-authority `frozen`/`frozen_reason` latch is deliberately excluded, so a
/// frozen doc never trips drift and a clean one never clears the latch (C1/C2).
fn journal_fields_eq(a: &EconomicAsset, b: &EconomicAsset) -> bool {
    a.object == b.object
        && a.title_owner == b.title_owner
        && a.scope == b.scope
        && a.assignment == b.assignment
        && a.maintenance_account == b.maintenance_account
        && a.catalog == b.catalog
        && a.encumbrances == b.encumbrances
        && a.acquisition == b.acquisition
        && a.lifecycle == b.lifecycle
        && a.last_journal_seq == b.last_journal_seq
        && a.revision == b.revision
}

fn live(w: &World, o: ObjId) -> bool {
    w.get(o).is_some_and(|x| !x.flags.contains(Flags::GOING))
}

fn principal_live(w: &World, p: &PrincipalId) -> bool {
    match p {
        PrincipalId::Object(o) => live(w, *o),
        // Org and system principals have no world object to lose.
        PrincipalId::Organization(_) | PrincipalId::System(_) => true,
    }
}

/// Whether every world principal an asset names still exists: the titled object,
/// the title owner, the assignee, and every creditor/claimant.
fn asset_is_live(w: &World, a: &EconomicAsset) -> bool {
    if !live(w, a.object) || !principal_live(w, &a.title_owner) {
        return false;
    }
    if let Some(x) = &a.assignment
        && !principal_live(w, &x.assigned_to)
    {
        return false;
    }
    a.encumbrances.iter().all(|e| match e {
        Encumbrance::Lien { creditor, .. } => principal_live(w, creditor),
        Encumbrance::Disputed { claimant, .. } => principal_live(w, claimant),
    })
}

/// Whether a vault's backing is still usable. A backing with no world object, or a
/// missing/`GOING` one, is unusable; a provider that answers `false` is unusable;
/// a provider that cannot answer (`None`) leaves the vault as it is — unknown is
/// never a freeze.
fn backing_usable(w: &World, providers: &CustodyTable, v: &Vault) -> bool {
    let Some(object) = v.backing.object() else { return false };
    if !live(w, object) {
        return false;
    }
    crate::custody::resolve(providers, w, &v.backing, None)
        .ok()
        .and_then(|p| p.backing_valid(w, &v.backing))
        .unwrap_or(true)
}

fn meta_u64(meta: &Option<serde_json::Value>, field: &str) -> u64 {
    meta.as_ref().and_then(|m| m.get(field)).and_then(serde_json::Value::as_u64).unwrap_or(0)
}
fn next_after(max: Option<u64>) -> u64 {
    max.map_or(0, |m| m + 1)
}

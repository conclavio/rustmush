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
//!
//! ## The freeze is DURABLE, and so are the floors
//!
//! Freezing is only a decision if it survives: this boot heals the very drift it
//! froze on and persists the healed docs, so a later boot re-deriving from those
//! docs alone would come up clean and spending. The latch therefore starts from the
//! stored `meta.frozen`, is OR-ed with everything this boot finds, and `meta` is
//! **persisted at the end of every boot** — which also makes the allocator floors
//! this boot just raised (past the journal, past every `approval:` key it saw,
//! readable or not) durable, instead of lasting until the next post happens to write
//! one. Only staff `@econ/unfreeze` clears the latch.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::module::{ModError, ModuleBootData, ModuleCtx};
use mush_core::{Flags, Ledger, LedgerChunk, ObjId, World, scan_contiguity};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::api::{self, META_KEY};
use crate::clock::CadenceClock;
use crate::custody::CustodyTable;
use crate::escalation::{
    ESCALATION_GC_AFTER_SECS, Escalation, EscalationKind, EscalationSubject, MAX_BOOT_ESCALATIONS,
    escalation_key,
};
use crate::contract::{ContractState, DepositRecord, EconomicContract, TypedCondition};
use crate::ids::{
    AccountId, ApprovalRequestId, CadenceId, ContractId, EscalationId, HoldId, InstrumentId,
    ObligationId, TransactionId, VaultId,
};
use crate::module::{BootParts, CustodyMark, EconomyStore};
use crate::obligation::{Obligation, ObligationCause, ObligationStatus, SuspendReason};
use crate::record::{
    Account, AccountStatus, ApprovalRequest, CustodyDirection, EconomicAsset, EconomicTransaction,
    Encumbrance, Hold, HoldEffectKind, IdempotencyRecord, InstrumentDef, Vault, VaultStatus,
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
    journal.restore_from_kv(&data.kv).map_err(|error| {
        ModError::msg(format!(
            "economy: authoritative journal restore refused: {error}"
        ))
    })?;

    // Parse the persisted docs. Instruments + accounts + vaults are containers (a
    // corrupt one fails the boot — proceeding would classify live money, or a live
    // custody binding, as absent). An asset doc is a projection, but a corrupt one
    // still fails: its `frozen` latch is doc authority and unreadable, and silently
    // un-freezing a recovering title is exactly the wrong direction.
    let mut instruments: BTreeMap<InstrumentId, InstrumentDef> = BTreeMap::new();
    let mut stored_accounts: BTreeMap<AccountId, Account> = BTreeMap::new();
    let mut stored_assets: BTreeMap<ObjId, EconomicAsset> = BTreeMap::new();
    let mut stored_vaults: BTreeMap<VaultId, Vault> = BTreeMap::new();
    let mut stored_approvals: BTreeMap<ApprovalRequestId, ApprovalRequest> = BTreeMap::new();
    let mut stored_holds: BTreeMap<HoldId, Hold> = BTreeMap::new();
    let mut stored_obligations: BTreeMap<ObligationId, Obligation> = BTreeMap::new();
    let mut stored_cadences: BTreeMap<CadenceId, CadenceClock> = BTreeMap::new();
    let mut stored_escalations: BTreeMap<EscalationId, Escalation> = BTreeMap::new();
    let mut stored_contracts: BTreeMap<ContractId, EconomicContract> = BTreeMap::new();
    let mut chunks: Vec<LedgerChunk> = Vec::new();
    let mut meta: Option<serde_json::Value> = None;
    // Every `approval:{id}` key seen, readable or not (the allocator floor).
    let mut max_approval_key: Option<u64> = None;
    // The same floor rule for E4b1's three new id spaces: an id that was minted must
    // never be minted again, even if the doc that carried it is now unreadable or has
    // been garbage-collected. (`hold:` is deliberately absent — the hold allocator is
    // floored from the JOURNAL alone, because GC deletes terminal hold docs and a
    // stale `Contract.escrow` id must never be re-minted onto a live reservation.)
    let mut max_obligation_key: Option<u64> = None;
    let mut max_cadence_key: Option<u64> = None;
    let mut max_escalation_key: Option<u64> = None;
    let mut max_contract_key: Option<u64> = None;

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
        } else if let Some(id) = key.strip_prefix("approval:") {
            // A **leaf**: an unreadable approval request costs at most a
            // re-submission, so it is skipped loudly rather than failing the boot
            // (§4.1). The warning names the key so staff can find the doc.
            let Ok(parsed) = id.parse::<u64>() else {
                tracing::warn!(%key, "economy: bad approval key; skipping");
                continue;
            };
            // The allocator floor is taken from the KEY, before the doc is parsed: a
            // corrupt doc is skipped, but its id was minted and must never be minted
            // again onto a different request (a re-mint would let a stale approval
            // doc, or a journal entry naming that id, attach to someone else's plan).
            max_approval_key = Some(max_approval_key.map_or(parsed, |m: u64| m.max(parsed)));
            match serde_json::from_value::<ApprovalRequest>(value.clone()) {
                Ok(r) if r.id == ApprovalRequestId(parsed) => {
                    stored_approvals.insert(r.id, r);
                }
                Ok(r) => tracing::warn!(%key, claimed = r.id.0, "economy: approval doc claims another id; skipping"),
                Err(e) => tracing::warn!(%key, error = %e, "economy: corrupt approval doc; skipping"),
            }
        } else if let Some(id) = key.strip_prefix("hold:") {
            // A **projection with no latch of its own** (D-E4-1): the journal carries
            // every field of a hold, so an unreadable doc costs nothing at all — it is
            // rebuilt from the journal exactly as a missing one is. Failing the boot
            // here would turn a lost read-side cache into an outage.
            let Ok(parsed) = id.parse::<u64>() else {
                tracing::warn!(%key, "economy: bad hold key; ignoring (rebuilt from the journal)");
                continue;
            };
            match serde_json::from_value::<Hold>(value.clone()) {
                Ok(h) if h.id == HoldId(parsed) => {
                    stored_holds.insert(h.id, h);
                }
                Ok(h) => tracing::warn!(%key, claimed = h.id.0, "economy: hold doc claims another id; rebuilding from the journal"),
                Err(e) => tracing::warn!(%key, error = %e, "economy: corrupt hold doc; rebuilding from the journal"),
            }
        } else if let Some(id) = key.strip_prefix("obligation:") {
            // A **container** (D-E4-5): obligation state IS the record, so a corrupt
            // one fails the boot rather than silently un-owing someone's rent.
            let id = id
                .parse::<u64>()
                .map_err(|_| ModError::msg(format!("economy: bad obligation key {key:?}")))?;
            max_obligation_key = Some(max_obligation_key.map_or(id, |m: u64| m.max(id)));
            let o = serde_json::from_value::<Obligation>(value.clone())
                .map_err(|e| ModError::msg(format!("economy: bad obligation doc {key:?}: {e}")))?;
            if o.id != ObligationId(id) {
                return Err(ModError::msg(format!("economy: obligation doc {key:?} claims id {}", o.id)));
            }
            stored_obligations.insert(o.id, o);
        } else if let Some(id) = key.strip_prefix("cadence:") {
            // Also a container (D-E4-2): every schedule on this clock reads its
            // position, so an unreadable one would silently re-date the campaign.
            let id = id
                .parse::<u64>()
                .map_err(|_| ModError::msg(format!("economy: bad cadence key {key:?}")))?;
            max_cadence_key = Some(max_cadence_key.map_or(id, |m: u64| m.max(id)));
            let c = serde_json::from_value::<CadenceClock>(value.clone())
                .map_err(|e| ModError::msg(format!("economy: bad cadence doc {key:?}: {e}")))?;
            if c.id != CadenceId(id) {
                return Err(ModError::msg(format!("economy: cadence doc {key:?} claims id {}", c.id)));
            }
            stored_cadences.insert(c.id, c);
        } else if let Some(id) = key.strip_prefix("contract:") {
            // A **container** (D-E4-5, §5): the contract record IS the agreement, so
            // a corrupt one fails the boot rather than silently un-owing a settlement
            // and stranding the escrow standing behind it.
            let id = id
                .parse::<u64>()
                .map_err(|_| ModError::msg(format!("economy: bad contract key {key:?}")))?;
            max_contract_key = Some(max_contract_key.map_or(id, |m: u64| m.max(id)));
            let c = serde_json::from_value::<EconomicContract>(value.clone())
                .map_err(|e| ModError::msg(format!("economy: bad contract doc {key:?}: {e}")))?;
            if c.id != ContractId(id) {
                return Err(ModError::msg(format!("economy: contract doc {key:?} claims id {}", c.id)));
            }
            stored_contracts.insert(c.id, c);
        } else if let Some(id) = key.strip_prefix("escalation:") {
            // A **leaf** (§4.6), exactly like an approval request: an unreadable alert
            // costs a staffer the alert, never the money it was about. Its id still
            // floors the allocator, so an acknowledgement can never resume the wrong
            // subject later.
            let Ok(parsed) = id.parse::<u64>() else {
                tracing::warn!(%key, "economy: bad escalation key; skipping");
                continue;
            };
            max_escalation_key = Some(max_escalation_key.map_or(parsed, |m: u64| m.max(parsed)));
            match serde_json::from_value::<Escalation>(value.clone()) {
                Ok(e) if e.id == EscalationId(parsed) => {
                    stored_escalations.insert(e.id, e);
                }
                Ok(e) => tracing::warn!(%key, claimed = e.id.0, "economy: escalation doc claims another id; skipping"),
                Err(e) => tracing::warn!(%key, error = %e, "economy: corrupt escalation doc; skipping"),
            }
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
    // How many journal ENTRIES carry a line on each account — `apply` bumps
    // `revision` exactly once per such entry, so `1 + touches` is the journal's own
    // revision for the account (invariant: revision is journal authority).
    let mut touches: BTreeMap<AccountId, u64> = BTreeMap::new();
    let mut snapshot_at_stored: BTreeMap<AccountId, i128> = BTreeMap::new();
    let mut journal_assets: BTreeMap<ObjId, EconomicAsset> = BTreeMap::new();
    let mut asset_at_stored: BTreeMap<ObjId, EconomicAsset> = BTreeMap::new();
    let mut last_custody: BTreeMap<ObjId, CustodyMark> = BTreeMap::new();
    let mut max_custody_vault: Option<u64> = None;
    let mut consumed_approval: BTreeMap<ApprovalRequestId, TransactionId> = BTreeMap::new();
    let mut max_consumed_approval: Option<u64> = None;
    // ---- E4b1: the reservations, folded exactly the way `apply` folds them ----
    // `journal_holds` is the last post-state per hold; `hold_at_stored` is the
    // post-state at the sequence the stored doc CLAIMS (the drift detector);
    // `running_held`/`held_at_stored` are the same pair for `Account.held_amount`,
    // anchored on `last_hold_seq` because an effect-only placement never moves
    // `last_journal_seq` (D11).
    let mut journal_holds: BTreeMap<HoldId, Hold> = BTreeMap::new();
    let mut hold_at_stored: BTreeMap<HoldId, Hold> = BTreeMap::new();
    let mut max_hold_id: Option<u64> = None;
    let mut hold_terminal_at: BTreeMap<HoldId, i64> = BTreeMap::new();
    let mut running_held: BTreeMap<AccountId, i128> = BTreeMap::new();
    let mut held_at_stored: BTreeMap<AccountId, i128> = BTreeMap::new();
    let mut last_hold_touch: BTreeMap<AccountId, u64> = BTreeMap::new();
    let mut transactions: BTreeMap<TransactionId, EconomicTransaction> = BTreeMap::new();
    let mut snapshot = EconomySnapshot::new();
    for def in instruments.values() {
        snapshot.put_instrument(def.clone());
    }
    let mut max_txid: u64 = 0;

    for (seq, tx) in &entries {
        let mut touched_here: BTreeSet<AccountId> = BTreeSet::new();
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
            touched_here.insert(line.account);
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
        // E4b1: reservations. Each effect carries the FULL post-state, so replay
        // rebuilds every hold — and every account's `held_amount` — from the journal
        // alone; a capture/release/expiry takes the WHOLE reservation off `held`,
        // which is exactly what `apply` does.
        for effect in &tx.hold_effects {
            let post = effect.hold.clone();
            if !stored_accounts.contains_key(&post.account) {
                return Err(ModError::msg(format!(
                    "economy: journal seq {seq} holds against unknown account {}; refusing to boot",
                    post.account
                )));
            }
            // E4b1 verify fold, B3: a hold effect touches its account exactly as a
            // line does — `apply` bumps `revision` once per entry over
            // `balance_deltas ∪ held_deltas`. Replay counted only the lines, so a
            // place/release pair left the re-derived revision BELOW the value that
            // was live before the crash; boot's `max` then kept the doc's higher
            // number for a doc that survived and silently rewound one that did not,
            // and an `expected_revisions` guard captured before the holds stopped
            // being `Stale` after the restart.
            touched_here.insert(post.account);
            max_hold_id = Some(max_hold_id.map_or(post.id.0, |m: u64| m.max(post.id.0)));
            let signed = if effect.kind == HoldEffectKind::Placed {
                post.amount.widen()
            } else {
                hold_terminal_at.insert(post.id, tx.posted_at);
                -post.amount.widen()
            };
            *running_held.entry(post.account).or_insert(0) += signed;
            last_hold_touch.insert(post.account, *seq);
            if stored_holds.get(&post.id).map(|h| h.last_journal_seq) == Some(*seq) {
                hold_at_stored.insert(post.id, post.clone());
            }
            if stored_accounts.get(&post.account).map(|a| a.last_hold_seq) == Some(*seq) {
                held_at_stored
                    .insert(post.account, *running_held.get(&post.account).expect("just inserted"));
            }
            journal_holds.insert(post.id, post);
        }
        // One revision per ENTRY per touched account — lines ∪ hold effects, the
        // exact set `apply` folds over (B3).
        for account in &touched_here {
            *touches.entry(*account).or_insert(0) += 1;
        }
        for effect in &tx.custody_effects {
            // E4b2: the depositor is the transaction's actor — journal authority, so
            // a contract's refund returns the object to whoever really put it there
            // rather than to whoever the record claims.
            last_custody.insert(effect.object, CustodyMark {
                vault: effect.vault,
                direction: effect.direction,
                by: tx.actor.clone(),
                at: tx.posted_at,
            });
            max_custody_vault = Some(max_custody_vault.map_or(effect.vault.0, |m: u64| m.max(effect.vault.0)));
        }
        // The journal is the authority on which requests were SPENT: a `Consumed`
        // doc can be lost, the entry that consumed it cannot. Keyed by ID (C3) —
        // healing by plan hash would let an unrelated request with a colliding
        // intent inherit the consumption.
        if let Some(id) = tx.consumed_approval {
            consumed_approval.insert(id, tx.id);
            max_consumed_approval = Some(max_consumed_approval.map_or(id.0, |m: u64| m.max(id.0)));
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
    //
    // The freeze latch starts from the **stored** one: a boot that froze also healed
    // the drifted docs it found and persisted them, so re-deriving from the docs
    // alone would come up clean on the very next boot and hand spending authority
    // back without any staff decision. `#[serde(default)]`-style tolerance (an
    // absent field reads `false`) keeps pre-E3 meta docs loading.
    let mut frozen = meta
        .as_ref()
        .and_then(|m| m.get("frozen"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if frozen {
        tracing::error!("economy: the stored whole-economy freeze latch is set; staying frozen (@econ/unfreeze clears it)");
    }
    // `held_amount`, re-derived: Σ of the account's still-`Held` reservations (§4.2).
    let mut derived_held: BTreeMap<AccountId, i128> = BTreeMap::new();
    for h in journal_holds.values() {
        if h.status.is_held() {
            *derived_held.entry(h.account).or_insert(0) += h.amount.widen();
        }
    }
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
        //
        // ...or claims a sequence at which the account has no line at all (E4b1
        // verify fold, B5): the asset and hold projections have always treated that
        // as an impossible doc and frozen on it, and the account arm has to, because
        // `unwrap_or(0)` reads exactly like a legitimately zero balance. A doc
        // claiming a sequence it never appears at, with a zero balance, was the one
        // shape that walked straight through. `last_journal_seq == 0` is the "never
        // touched" sentinel and is not a claim.
        if acct.last_journal_seq != 0 && !snapshot_at_stored.contains_key(&id) {
            tracing::error!(account = id.0, stored_seq = acct.last_journal_seq, "economy: account projection claims a sequence it never appears at; freezing");
            frozen = true;
        }
        let claimed = snapshot_at_stored.get(&id).copied().unwrap_or(0);
        if Amount::from_widened(claimed) != Some(acct.posted_balance) {
            tracing::error!(account = id.0, "economy: stored balance drifts from the journal at its claimed seq; freezing");
            frozen = true;
        }
        // ---- E4b1: `held_amount`, on its OWN sequence anchor (§4.2, D11) ----
        //
        // The same three directional rules as `posted_balance`, asked at
        // `last_hold_seq` rather than `last_journal_seq`: a `Place`/`Release`/`Expire`
        // is an effect-only entry that never moves the money anchor, so comparing the
        // reservation projection there would read every placement as drift.
        let derived_held_amount =
            i64::try_from(derived_held.get(&id).copied().unwrap_or(0)).ok().map(Amount::from_minor);
        let hold_touch = last_hold_touch.get(&id).copied().unwrap_or(0);
        let mut held_ok = true;
        if acct.last_hold_seq > journal_max.unwrap_or(0) && acct.last_hold_seq != 0 {
            tracing::error!(account = id.0, stored_seq = acct.last_hold_seq, journal_max = ?journal_max, "economy: held projection ahead of the journal (lost tail); freezing");
            frozen = true;
        }
        // The third rule, mirrored from the asset and hold arms (B5): a doc claiming
        // a hold sequence at which this account has NO hold effect is impossible, and
        // `held_amount: 0` is precisely where the comparison below goes blind —
        // `unwrap_or(0)` cannot tell "no effect there" from "reserved nothing there".
        if acct.last_hold_seq != 0 && !held_at_stored.contains_key(&id) {
            tracing::error!(account = id.0, stored_seq = acct.last_hold_seq, "economy: held projection claims a sequence it never appears at; freezing");
            frozen = true;
        }
        let claimed_held = held_at_stored.get(&id).copied().unwrap_or(0);
        if Amount::from_widened(claimed_held) != Some(acct.held_amount) {
            tracing::error!(account = id.0, "economy: stored held_amount drifts from the journal at its claimed hold seq; freezing");
            frozen = true;
        }
        let held_moved = match derived_held_amount {
            Some(h) => {
                let moved = acct.held_amount != h || acct.last_hold_seq != hold_touch;
                acct.held_amount = h;
                acct.last_hold_seq = hold_touch;
                moved
            }
            None => {
                tracing::error!(account = id.0, "economy: replayed held_amount does not fit i64; freezing");
                frozen = true;
                held_ok = false;
                false
            }
        };
        if !held_ok {
            acct.status = crate::record::AccountStatus::Frozen;
        }
        // Adopt the journal-derived balance (heals a benign lag; a no-op when clean),
        // and with it the journal-derived REVISION. Replay never re-derived
        // `revision`, so a lagging doc healed its balance forward and left its
        // revision behind — and an `expected_revisions` guard written against the
        // pre-reboot value then passed against a state it was never checked against.
        // `max` because `revision` is also bumped by doc-authority backstops (the
        // destruction freeze), which the journal cannot see: never LOWER it.
        let journal_revision = 1 + touches.get(&id).copied().unwrap_or(0);
        let stale_seq = acct.last_journal_seq != last_touch.get(&id).copied().unwrap_or(0);
        if acct.posted_balance != derived || stale_seq || acct.revision < journal_revision || held_moved {
            acct.posted_balance = derived;
            acct.last_journal_seq = last_touch.get(&id).copied().unwrap_or(0);
            acct.revision = acct.revision.max(journal_revision);
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
                // Whether THIS title is the one that latched the whole-economy
                // freeze — it earns its own per-asset latch, so the trace of which
                // record needs staff attention survives the recovery boot (the
                // whole-economy freeze is cleared in one motion; a per-asset one is
                // cleared per record, deliberately).
                let mut drifted = false;
                if doc.last_journal_seq > journal_max.unwrap_or(0) {
                    tracing::error!(object = object.0, stored_seq = doc.last_journal_seq, journal_max = ?journal_max, "economy: asset projection ahead of the journal (lost tail); freezing");
                    drifted = true;
                } else {
                    match asset_at_stored.get(object) {
                        // The doc claims a sequence at which this object really had an
                        // effect: its journal-authority fields must match it exactly.
                        Some(at) if journal_fields_eq(&doc, at) => {}
                        Some(_) => {
                            tracing::error!(object = object.0, "economy: asset projection drifts from the journal at its claimed seq; freezing");
                            drifted = true;
                        }
                        // ...or a sequence at which it had none: an impossible doc.
                        None => {
                            tracing::error!(object = object.0, stored_seq = doc.last_journal_seq, "economy: asset projection claims a sequence it never appears at; freezing");
                            drifted = true;
                        }
                    }
                }
                frozen |= drifted;
                // Adopt the journal head (heals a benign lag), keeping the latch.
                let mut a = derived.clone();
                a.frozen = doc.frozen || drifted;
                a.frozen_reason = if doc.frozen {
                    doc.frozen_reason.clone()
                } else if drifted {
                    Some(SmolStr::new_static("drift"))
                } else {
                    None
                };
                if drifted || !journal_fields_eq(&doc, derived) {
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

    // ---- E4b1: hold projections (§4.2) -----------------------------------
    //
    // The asset rules, minus the latch: a [`Hold`] has **no doc-authority field at
    // all**, so the whole record is compared and a missing (or unreadable) doc is
    // rebuilt SILENTLY — there is nothing a lost hold doc could lose. Only a doc that
    // disagrees with the journal at its own claimed sequence, or one ahead of the
    // journal, is drift, and drift freezes.
    let mut healed_holds: Vec<Hold> = Vec::new();
    for (id, derived) in &journal_holds {
        match stored_holds.remove(id) {
            None => {
                tracing::warn!(hold = id.0, "economy: hold doc missing; rebuilt from the journal");
                healed_holds.push(derived.clone());
                snapshot.put_hold(derived.clone());
            }
            Some(doc) => {
                let mut drifted = false;
                if doc.last_journal_seq > journal_max.unwrap_or(0) {
                    tracing::error!(hold = id.0, stored_seq = doc.last_journal_seq, journal_max = ?journal_max, "economy: hold projection ahead of the journal (lost tail); freezing");
                    drifted = true;
                } else {
                    match hold_at_stored.get(id) {
                        Some(at) if *at == doc => {}
                        Some(_) => {
                            tracing::error!(hold = id.0, "economy: hold projection drifts from the journal at its claimed seq; freezing");
                            drifted = true;
                        }
                        None => {
                            tracing::error!(hold = id.0, stored_seq = doc.last_journal_seq, "economy: hold projection claims a sequence it never appears at; freezing");
                            drifted = true;
                        }
                    }
                }
                frozen |= drifted;
                if drifted || doc != *derived {
                    healed_holds.push(derived.clone());
                }
                snapshot.put_hold(derived.clone());
            }
        }
    }
    // A hold doc with no journal history is not a reservation (the asset rule, C6):
    // loading one would let a hand-written doc freeze someone's money.
    for id in stored_holds.keys() {
        tracing::warn!(hold = id.0, "economy: hold doc has no journal history; NOT loaded");
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

    // ---- E3b2: approval requests (leaf docs, §4.1) ------------------------
    //
    // Three directional rules, all of which move a request toward "not usable":
    // a Pending request the journal proves was spent becomes `Consumed` (healed by
    // ID, never by hash); one past its TTL becomes `Expired`; and a long-dead one is
    // garbage-collected. Nothing here ever revives a request.
    let now = ctx.w.now();
    let mut healed_approvals: Vec<ApprovalRequest> = Vec::new();
    let mut dropped_approvals: Vec<ApprovalRequestId> = Vec::new();
    let max_stored_approval: Option<u64> = stored_approvals.keys().map(|id| id.0).max();
    for (id, mut r) in stored_approvals {
        if r.status == crate::record::ApprovalStatus::Pending {
            if let Some(tx) = consumed_approval.get(&id) {
                tracing::warn!(approval = id.0, transaction = tx.0, "economy: approval doc lags the journal; marking it consumed");
                r.status = crate::record::ApprovalStatus::Consumed { transaction: *tx };
                r.revision += 1;
                healed_approvals.push(r.clone());
            } else if now >= r.expires_at {
                r.status = crate::record::ApprovalStatus::Expired;
                r.revision += 1;
                healed_approvals.push(r.clone());
            }
        }
        // GC: a dead request stops being an audit trail long before it stops being
        // clutter. `Consumed` requests are NEVER collected — they are the other end
        // of a journal entry's `consumed_approval`.
        let dead = matches!(
            r.status,
            crate::record::ApprovalStatus::Expired | crate::record::ApprovalStatus::Cancelled
        );
        if dead && now.saturating_sub(r.expires_at) > crate::APPROVAL_GC_AFTER_SECS {
            dropped_approvals.push(id);
            continue;
        }
        snapshot.put_approval(r);
    }

    // ---- E4b1: cadence clocks (containers) -------------------------------
    for (_, c) in stored_cadences {
        snapshot.put_cadence(c);
    }

    // ---- E4b1: escalation records (leaf docs, §4.6) -----------------------
    //
    // One directional rule, and it only ever moves a record toward "gone": an
    // ACKNOWLEDGED alert older than the retention window is collected. An open one is
    // never touched — it is the only thing still saying a human has to look.
    let mut escalations: BTreeMap<EscalationId, Escalation> = BTreeMap::new();
    let mut dropped_escalations: Vec<EscalationId> = Vec::new();
    for (id, e) in stored_escalations {
        let collectable = e
            .acked_at
            .is_some_and(|at| now.saturating_sub(at) > ESCALATION_GC_AFTER_SECS);
        if collectable {
            dropped_escalations.push(id);
            continue;
        }
        escalations.insert(id, e);
    }
    // The escalation allocator has to be live BEFORE obligation reconciliation, which
    // may open records of its own.
    let mut next_escalation = meta_u64(&meta, "next_escalation_id")
        .max(next_after(max_escalation_key))
        .max(next_after(escalations.keys().map(|i| i.0).max()));
    // Boot's own level-2 records, capped at `MAX_BOOT_ESCALATIONS` across BOTH
    // reconciliation passes: a broken world must not mint an unbounded alert storm,
    // and the subject is stopped either way.
    let mut opened_escalations: Vec<Escalation> = Vec::new();

    // ---- E4b2: contracts (containers, §5) --------------------------------
    //
    // Contracts load BEFORE obligations, because a `Contract`-caused obligation's
    // own reconciliation asks whether its contract is still live — E4b1 answered
    // "no record, so no" for every one of them, which was the fail-closed reading
    // and is now the real question.
    //
    // Three journal checks, each with a different answer because each means
    // something different:
    //
    // - an `Active`/`ReadyToSettle` contract whose escrow is not `Held` in the
    //   journal is an agreement standing on money that is gone → **`Disputed`** and
    //   a loud log (§15.2 "disable and quarantine, reserve no value"). Nothing is
    //   captured, nothing is released; the escrow, whatever is left of it, freezes.
    // - a `Settled { transaction }` whose transaction the journal does not have is
    //   **drift**: a settlement the journal never saw is a doc claiming money moved.
    //   That is the whole-economy freeze, with the level-2 companion record E4b1
    //   opens for one.
    // - a `Settled` transaction the journal has but which was **reversed** →
    //   `Disputed`. Invariant 28 makes that unreachable (a capture-carrying entry
    //   cannot be reversed), which is exactly why it is checked: an unreachable
    //   state that shows up is evidence about the journal, not about the contract.
    let mut healed_contracts: Vec<EconomicContract> = Vec::new();
    for (_, mut c) in stored_contracts {
        let mut quarantine: Option<String> = None;
        // E4b2 verify fold, B6: **a stored surface is not evidence.** `Offered` on
        // disk only means it passed `check_offerable` at some past moment, against a
        // past code version, before whatever edit or migration produced the bytes now
        // on disk. A record whose `After` anchors form a cycle, whose split cannot be
        // paid, whose deadline names a clock nothing resolves, or whose payer is not a
        // party can never settle — and its escrow would sit until a deadline that may
        // itself be unresolvable, with nothing saying why. Re-run the structural half
        // here (the world half needs a snapshot boot is still assembling) and
        // quarantine what it refuses; the escrow is untouched either way.
        if !c.state.is_terminal()
            && let Err(e) = crate::contract::check_structure(&c)
        {
            quarantine = Some(format!(
                "contract #{} could not settle as stored ({e}); its surface is quarantined",
                c.id
            ));
        }
        match &c.state {
            ContractState::Active | ContractState::ReadyToSettle => {
                let broken = c.escrow.iter().any(|id| {
                    !journal_holds.get(id).is_some_and(|h| h.status.is_held())
                });
                // E4b2 verify fold, B1: **finish the transition before disputing it.**
                // A terminal transition is a post under the contract's own
                // deterministic key followed by a state write, so a lost doc write
                // leaves exactly this shape: escrow released (or captured) in the
                // journal, and a record still claiming to be live. Disputing that is
                // wrong twice over — it pages a human about a contract that is
                // finished, and it leaves the state machine claiming money is reserved
                // that the journal says is not. The key resolves the question with no
                // guessing at all.
                if broken && quarantine.is_none() {
                    match finished_transition(&c, &snapshot, &transactions) {
                        Some(state) => {
                            tracing::info!(
                                contract = c.id.0,
                                state = ?state.kind(),
                                "economy: a contract's terminal transition had posted but its doc \
                                 write was lost; finishing it from the idempotency index"
                            );
                            c.state = state;
                            c.pending = None;
                            c.revision += 1;
                            healed_contracts.push(c.clone());
                        }
                        None => {
                            quarantine = Some(format!(
                                "contract #{}'s escrow is not held in the journal, and none of its \
                                 terminal keys resolves to a transaction",
                                c.id
                            ));
                        }
                    }
                }
            }
            ContractState::Settled { transaction } => {
                // E4b2 verify fold, B5: existence is not identity. A doc naming a
                // transaction the journal HAS, but which is some other contract's
                // settlement (or any other post), is a record claiming a payout that
                // never happened to it — the same class of drift as naming a
                // transaction that does not exist, and it must not read as clean.
                let mine = crate::contract::contract_op_key(c.id, "settle", c.amendments);
                let claims_another = transactions
                    .get(transaction)
                    .is_some_and(|tx| tx.idempotency != mine);
                if !transactions.contains_key(transaction) || claims_another {
                    tracing::error!(
                        contract = c.id.0,
                        transaction = transaction.0,
                        claims_another,
                        "economy: a settled contract names a transaction that is not its own \
                         settlement; freezing the economy"
                    );
                    frozen = true;
                    if opened_escalations.len() < MAX_BOOT_ESCALATIONS
                        && let Some(id) = mint_escalation(&mut next_escalation)
                    {
                        opened_escalations.push(Escalation::page(
                            id,
                            EscalationKind::EconomyFrozen,
                            EscalationSubject::Contract(c.id),
                            format!(
                                "contract #{} says it settled as transaction #{transaction}, and \
                                 that is not this contract's settlement. A settlement the journal \
                                 does not attribute here is drift, so the whole economy is frozen.",
                                c.id
                            ),
                            vec![
                                format!("@contract/show #{}", c.id),
                                "@econ/audit".to_string(),
                                "@econ/unfreeze   (only after the journal is understood)".to_string(),
                            ],
                            now,
                        ));
                    }
                } else if snapshot.is_reversed(*transaction) {
                    quarantine = Some(format!(
                        "contract #{}'s settlement transaction #{transaction} was reversed",
                        c.id
                    ));
                }
            }
            ContractState::Draft
            | ContractState::Offered
            | ContractState::Disputed
            | ContractState::Cancelled
            | ContractState::Expired => {}
        }
        if let Some(reason) = quarantine {
            tracing::error!(contract = c.id.0, %reason, "economy: boot quarantined a contract");
            c.state = ContractState::Disputed;
            c.pending = None;
            c.revision += 1;
            healed_contracts.push(c.clone());
            if opened_escalations.len() < MAX_BOOT_ESCALATIONS
                && let Some(id) = mint_escalation(&mut next_escalation)
            {
                opened_escalations.push(Escalation::page(
                    id,
                    EscalationKind::BootReconciliation,
                    EscalationSubject::Contract(c.id),
                    reason,
                    vec![
                        format!("@contract/show #{}", c.id),
                        format!("@contract/resolve #{}=refund", c.id),
                        format!("@contract/resolve #{}=settle", c.id),
                    ],
                    now,
                ));
            }
        }
        // The depositor of every live `DepositInto`, from the journal's last-custody
        // map (§5). Recording it here is what lets a refund put the object back: the
        // custody lock has no exception, so the contract's own unwind is the only
        // route out, and it has to know whose the thing was.
        if c.state.is_live() {
            let deposits = derive_deposits(&c, &last_custody);
            if deposits != c.deposits {
                c.deposits = deposits;
                c.revision += 1;
                healed_contracts.push(c.clone());
            }
        }
        // `put_contract` re-derives the custody locks from the record's own
        // `DepositInto` conditions, so the locks come back exactly as the live states
        // describe them and a terminal contract holds nothing.
        snapshot.put_contract(c);
    }

    // ---- E4b2 verify fold, B2: orphan escrow ------------------------------
    //
    // A hold the journal says is **`Held`** with `purpose: Escrow { contract }`, that
    // the contract's own `escrow` list does not name, is money nothing in the world
    // can reach: `WorldAuth::authorize_hold` refuses every op on an escrow hold, and
    // `ContractAuth` only ever touches what `qualifying_escrow` returns — which reads
    // that list. It is exactly what a lost `@contract/accept` doc write leaves behind
    // (which is why the accept now rides the post's own batch), and a boot is the one
    // place that can see both halves at once.
    //
    // Three answers, because the three cases are different:
    //
    // - **non-terminal and it qualifies** — the hold matches an unclaimed
    //   consideration, so it IS this contract's escrow: adopt it. Loud, no page: the
    //   record is now right and the contract can settle or refund normally.
    // - **terminal** — the money has to come back, and the contract's own unwind is
    //   the only thing that may release it. Adopt it, advance the key **generation**
    //   (the old refund key is spent, and reusing it would be an idempotency HIT that
    //   hands nothing back), and set `pending` so the sweep posts the unwind under a
    //   fresh key and lands the same terminal state again.
    // - **anything else** — a settled contract, a hold matching no consideration, a
    //   contract with no record at all: a human decides. Page, with the command.
    let orphans: Vec<Hold> = journal_holds
        .values()
        .filter(|h| h.status.is_held())
        .filter(|h| match h.purpose {
            crate::record::HoldPurpose::Escrow { contract } => snapshot
                .contract(contract)
                .is_none_or(|c| !c.escrow.contains(&h.id)),
            _ => false,
        })
        .cloned()
        .collect();
    for hold in orphans {
        let crate::record::HoldPurpose::Escrow { contract } = hold.purpose else { continue };
        let Some(mut c) = snapshot.contract(contract).cloned() else {
            tracing::error!(hold = hold.id.0, contract = contract.0, "economy: an escrow hold names a contract with no record");
            if opened_escalations.len() < MAX_BOOT_ESCALATIONS
                && let Some(id) = mint_escalation(&mut next_escalation)
            {
                opened_escalations.push(Escalation::page(
                    id,
                    EscalationKind::BootReconciliation,
                    EscalationSubject::Contract(contract),
                    format!(
                        "hold #{} is escrow for contract #{contract}, and there is no such contract                          record; the money is reserved and nothing can release it",
                        hold.id
                    ),
                    vec![format!("@contract/show #{contract}"), "@econ/audit".to_string()],
                    now,
                ));
            }
            continue;
        };
        // Does it qualify? Ask the real predicate, with the hold provisionally listed.
        let mut probe = c.clone();
        probe.escrow.push(hold.id);
        let qualifies = crate::contract::qualifying_escrow(&probe, &snapshot)
            .holds
            .iter()
            .any(|h| h.id == hold.id);
        let terminal = c.state.is_terminal();
        if qualifies && !terminal {
            tracing::warn!(hold = hold.id.0, contract = contract.0, "economy: adopting an orphaned escrow hold into its contract");
            c.escrow.push(hold.id);
            c.revision += 1;
            healed_contracts.push(c.clone());
            snapshot.put_contract(c);
            continue;
        }
        if qualifies && matches!(c.state, ContractState::Cancelled | ContractState::Expired) {
            tracing::error!(hold = hold.id.0, contract = contract.0, "economy: an ended contract still holds escrow; scheduling its own unwind under a fresh key generation");
            c.escrow.push(hold.id);
            c.amendments = c.amendments.saturating_add(1);
            c.pending = Some(crate::contract::PendingTerminal {
                kind: if c.state == ContractState::Cancelled {
                    crate::contract::PendingKind::Cancel
                } else {
                    crate::contract::PendingKind::Expire
                },
                attempts: 0,
                since: now,
            });
            c.revision += 1;
            healed_contracts.push(c.clone());
            snapshot.put_contract(c);
            continue;
        }
        tracing::error!(hold = hold.id.0, contract = contract.0, state = ?c.state.kind(), "economy: an escrow hold belongs to no consideration its contract names");
        if opened_escalations.len() < MAX_BOOT_ESCALATIONS
            && let Some(id) = mint_escalation(&mut next_escalation)
        {
            opened_escalations.push(Escalation::page(
                id,
                EscalationKind::BootReconciliation,
                EscalationSubject::Contract(contract),
                format!(
                    "hold #{} is escrow for contract #{contract} but matches nothing that contract                      agreed to pay; the money is reserved and only a human can decide where it goes",
                    hold.id
                ),
                vec![
                    format!("@contract/show #{contract}"),
                    format!("@contract/resolve #{contract}=refund"),
                    "@econ/audit".to_string(),
                ],
                now,
            ));
        }
    }

    // ---- E4b1: obligations (containers, §4.2) ----------------------------
    //
    // Boot never guesses and never deletes: an obligation whose debtor, creditor, or
    // either account can no longer carry it is **suspended** — out of the due pool, so
    // the sweep stops attempting it — and, per the ladder (§4.6, D-E4-14), paged with
    // the command that fixes it already written out. Nothing is charged, nothing is
    // lost, and a staffer sees the question rather than a silent stall.
    let mut healed_obligations: Vec<Obligation> = Vec::new();
    for (_, mut o) in stored_obligations {
        if let Some(reason) = obligation_unusable(ctx.w, &snapshot, &o)
            && o.status.in_due_pool()
        {
            {
                tracing::error!(obligation = o.id.0, %reason, "economy: boot suspended an obligation it cannot settle");
                let minted = (opened_escalations.len() < MAX_BOOT_ESCALATIONS)
                    .then(|| mint_escalation(&mut next_escalation))
                    .flatten();
                let suspend = if let Some(id) = minted {
                    let record = Escalation::page(
                        id,
                        EscalationKind::BootReconciliation,
                        EscalationSubject::Obligation(o.id),
                        reason,
                        boot_advice(&o),
                        now,
                    );
                    opened_escalations.push(record);
                    SuspendReason::Escalated(id)
                } else {
                    // Past the cap the subject is still stopped and the reason still
                    // logged: a broken world must not mint an unbounded alert storm.
                    SuspendReason::Staff { reason: SmolStr::new_static("boot-reconciliation") }
                };
                o.status = ObligationStatus::Suspended { reason: suspend };
                o.revision += 1;
                healed_obligations.push(o.clone());
            }
        }
        snapshot.put_obligation(o);
    }
    for e in &opened_escalations {
        escalations.insert(e.id, e.clone());
    }

    // ---- E3: custody reconciliation (audit-only; never derives a location) --
    let deposited: Vec<(ObjId, VaultId)> = last_custody
        .iter()
        .filter(|(_, mark)| mark.direction == CustodyDirection::Deposit)
        .map(|(o, mark)| (*o, mark.vault))
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
    //
    // E4b1 verify fold, B4a: record WHICH accounts this latch froze. Recovery used to
    // be a choice between unfreezing every account by hand and unfreezing them all —
    // which silently cleared the accounts frozen for their own reasons (a destroyed
    // owner, a balance that would not fit `i64`) and erased the list of what was
    // really broken. The list accumulates across every boot the latch survives,
    // because a second recovery boot sees the accounts already `Frozen` and would
    // otherwise record nothing at all; only `@econ/unfreeze` clears it.
    let mut boot_frozen: Vec<AccountId> = meta
        .as_ref()
        .and_then(|m| m.get("boot_frozen"))
        .and_then(serde_json::Value::as_array)
        .map(|xs| xs.iter().filter_map(serde_json::Value::as_u64).map(AccountId).collect())
        .unwrap_or_default();
    if frozen {
        let ids: Vec<AccountId> = snapshot.accounts().map(|a| a.id).collect();
        for id in ids {
            if let Some(a) = snapshot.account_mut(id)
                && a.status == crate::record::AccountStatus::Active
            {
                a.status = crate::record::AccountStatus::Frozen;
                boot_frozen.push(id);
            }
        }
        boot_frozen.sort_unstable();
        boot_frozen.dedup();
    }

    // Allocators: meta is authoritative but floored at one past the journal-derived
    // max so a stale meta never re-mints a live id.
    let next_account = meta_u64(&meta, "next_account_id").max(next_after(snapshot.accounts().map(|a| a.id.0).max()));
    let next_transaction = meta_u64(&meta, "next_transaction_id").max(if entries.is_empty() { 0 } else { max_txid + 1 });
    let next_vault = meta_u64(&meta, "next_vault_id")
        .max(next_after(max_custody_vault))
        .max(next_after(snapshot.vaults().map(|v| v.id.0).max()));
    // C4: floor the approval allocator past every id the JOURNAL has seen and every
    // stored doc — including the ones just garbage-collected, and the ones whose doc
    // failed to deserialize (skipped as leaves, but their ids were minted), so a
    // collected or unreadable id is never re-minted onto a different request.
    let next_approval = meta_u64(&meta, "next_approval_id")
        .max(next_after(max_consumed_approval))
        .max(next_after(max_stored_approval))
        .max(next_after(max_approval_key));
    // D14 / the E4a handoff: the hold allocator is floored from the **journal**, and
    // from nothing else. Surviving `hold:{id}` docs are the wrong source — GC deletes
    // terminal ones, so a floor taken from the docs would drop back below a
    // garbage-collected id and re-mint it under a stale `Contract.escrow` reference,
    // attaching live escrow to a dead contract. `HoldIdReused` in the engine is the
    // backstop for that, never the mechanism.
    let next_hold = meta_u64(&meta, "next_hold_id").max(next_after(max_hold_id));
    let next_obligation =
        meta_u64(&meta, "next_obligation_id").max(next_after(max_obligation_key));
    let next_cadence = meta_u64(&meta, "next_cadence_id")
        .max(next_after(max_cadence_key))
        .max(next_after(snapshot.cadences().map(|c| c.id.0).max()));
    let next_escalation = next_escalation.max(meta_u64(&meta, "next_escalation_id"));
    let next_contract = meta_u64(&meta, "next_contract_id")
        .max(next_after(max_contract_key))
        .max(next_after(snapshot.contracts().map(|c| c.id.0).max()));
    let sweep_contract_cursor = meta_u64(&meta, "sweep_contract_cursor");
    let last_sweep_real = meta_i64(&meta, "last_sweep_real");
    let last_sweep_ic = meta_i64(&meta, "last_sweep_ic");

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
    for r in &healed_approvals {
        api::persist_approval(ctx.out, r);
    }
    for id in &dropped_approvals {
        api::delete_key(ctx.out, api::approval_key(*id));
    }
    for h in &healed_holds {
        api::persist_hold(ctx.out, h);
    }
    for o in &healed_obligations {
        api::persist_obligation(ctx.out, o);
    }
    for c in &healed_contracts {
        api::persist_contract(ctx.out, c);
    }
    for e in &opened_escalations {
        api::persist_escalation(ctx.out, e);
    }
    for id in &dropped_escalations {
        api::delete_key(ctx.out, escalation_key(*id));
    }

    let store = EconomyStore::from_parts(BootParts {
        snapshot,
        journal,
        transactions,
        escalations,
        hold_terminal_at,
        next_account,
        next_transaction,
        next_vault,
        next_approval,
        next_hold,
        next_obligation,
        next_cadence,
        next_escalation,
        next_contract,
        last_custody,
        sweep_contract_cursor,
        last_sweep_real,
        last_sweep_ic,
        // S3: the boot marker. Stamped at the END of every boot, so a sweep gap that
        // contains it provably contains a restart — evidence, not an inference from
        // how long the gap was.
        last_boot_real: now,
        boot_frozen,
        frozen,
    });
    // Persist `meta` at the END of every boot, always. Two things ride on it and
    // both were previously RAM-only until the next post happened to write one: the
    // whole-economy **freeze latch** (a boot that froze, healed, and never wrote it
    // came up spending on the next boot), and the allocator **floors** boot just
    // raised past the journal (a stale meta plus a lost doc would otherwise re-mint
    // a live id one boot later, when the doc that raised the floor is gone).
    api::persist_meta(ctx.out, store.meta_doc());
    Ok(store)
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

/// Whether a principal still exists in the world (an org or system principal has no
/// world object to lose). `pub(crate)` since the E4b1 verify fold: the sweep asks the
/// same question before recommending `@account/unfreeze` (B4b).
pub(crate) fn principal_live(w: &World, p: &PrincipalId) -> bool {
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
        // A contract-placed claim (E4a) is live while its CONTRACT is; a destroyed
        // lessor cannot strand the asset, because the contract is the party that
        // releases the claim. E4b2's boot reconciles the contract itself.
        Encumbrance::Lease { .. } | Encumbrance::Collateral { .. } => true,
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

/// Why boot cannot leave an obligation in the due pool, or `None` if it can (§4.2).
///
/// Four questions, all of them about things outside the economy's own records: is
/// each party still a live world object, does each account still exist and still
/// take postings, and — for a contract-caused obligation — is its contract still
/// live? The answer is a **sentence**, because it becomes the escalation's diagnosis
/// and the first thing a staffer reads.
fn obligation_unusable(w: &World, snapshot: &EconomySnapshot, o: &Obligation) -> Option<String> {
    if !principal_live(w, &o.debtor) {
        return Some(format!("obligation #{}'s debtor no longer exists", o.id));
    }
    if !principal_live(w, &o.creditor) {
        return Some(format!("obligation #{}'s creditor no longer exists", o.id));
    }
    // E4b1 verify fold, B6: a schedule whose clock cannot be resolved is not "not yet
    // due", it is **never** due. `Clocks::now_on` answers `None` for a
    // `Cadence(id)` with no record and every consumer reads that as "not yet", so a
    // lease pointing at a deleted campaign clock sat `Active` in the due pool
    // forever, charging nothing and telling nobody. It is exactly the shape the other
    // three questions here exist for: outside the record, unfixable by the sweep, and
    // a decision only a human can make.
    if let crate::clock::EconomicClock::Cadence(c) = o.schedule.clock
        && snapshot.cadence(c).is_none()
    {
        return Some(format!(
            "obligation #{}'s schedule runs on cadence clock #{c}, which has no record: no period \
             can ever fall due on it",
            o.id
        ));
    }
    for (label, id) in [("debit", o.debit_account), ("credit", o.credit_account)] {
        match snapshot.account(id) {
            None => return Some(format!("obligation #{}'s {label} account #{id} is gone", o.id)),
            Some(a) if a.status == AccountStatus::Closed => {
                return Some(format!("obligation #{}'s {label} account #{id} is closed", o.id));
            }
            Some(_) => {}
        }
    }
    // A contract-caused obligation outlives its contract only as a record: a lease
    // whose contract is cancelled must stop charging rent. (Contracts load in E4b2;
    // until then a `Contract` cause has no live record and is suspended, which is the
    // fail-closed reading and the same answer E4b2 will give for a missing one.)
    if let ObligationCause::Contract(c) = &o.cause {
        let live = snapshot.contract(*c).is_some_and(|k| {
            matches!(
                k.state,
                crate::contract::ContractState::Active
                    | crate::contract::ContractState::ReadyToSettle
                    | crate::contract::ContractState::Settled { .. }
            )
        });
        if !live {
            return Some(format!("obligation #{}'s contract #{c} is no longer live", o.id));
        }
    }
    None
}

/// The recommended commands for an obligation a human must now decide about (§4.6).
/// The staffer is going to paste the alert into an LLM anyway, so the alert carries
/// the answer: repoint it at a working account, or end it.
///
/// A schedule on a **missing cadence clock** gets its own advice (B6) — repointing an
/// account fixes nothing when the problem is that no period can fall due at all, and
/// an alert whose recommended command cannot work is worse than no recommendation.
fn boot_advice(o: &Obligation) -> Vec<String> {
    let id = o.id;
    if let crate::clock::EconomicClock::Cadence(c) = o.schedule.clock {
        return vec![
            format!("@obligation/show #{id}   (its schedule runs on cadence clock #{c})"),
            format!("@obligation/cancel #{id}"),
            "@econ/ack #<this alert>   (resumes it once the clock exists again)".to_string(),
        ];
    }
    vec![
        format!("@obligation/repoint #{id}=credit <account id>"),
        format!("@obligation/repoint #{id}=debit <account id>"),
        "@econ/ack #<this alert>   (resumes it once repointed)".to_string(),
        format!("@obligation/cancel #{id}"),
    ]
}

fn meta_u64(meta: &Option<serde_json::Value>, field: &str) -> u64 {
    meta.as_ref().and_then(|m| m.get(field)).and_then(serde_json::Value::as_u64).unwrap_or(0)
}
fn meta_i64(meta: &Option<serde_json::Value>, field: &str) -> i64 {
    meta.as_ref().and_then(|m| m.get(field)).and_then(serde_json::Value::as_i64).unwrap_or(0)
}
/// One past the largest id seen, **saturating** (E4b1 verify fold, B1).
///
/// `m + 1` panicked in a debug build (and wrapped to `0` in a release one) on a
/// single hand-written or corrupt key at `u64::MAX` — `escalation:18446744073709551615`
/// was enough to make the world refuse to boot, or worse, to floor every allocator at
/// zero and re-mint live ids. Saturating parks the allocator at the ceiling, where
/// [`EconomyStore::alloc_escalation`] and friends now refuse to mint (B2) instead of
/// overflowing: the safe failure is "no new ids", never "the same id twice".
fn next_after(max: Option<u64>) -> u64 {
    max.map_or(0, |m| m.saturating_add(1))
}

/// Mint one boot escalation id, or `None` at the ceiling (E4b1 verify fold, B1/B2).
/// The allocator floor **saturates**, so at `u64::MAX` this refuses rather than
/// wrapping a live id back into use; the subject is still quarantined either way.
fn mint_escalation(next: &mut u64) -> Option<EscalationId> {
    if *next == u64::MAX {
        tracing::error!("economy: the escalation allocator is at its ceiling; boot opens no record");
        return None;
    }
    let id = EscalationId(*next);
    *next = next.saturating_add(1);
    Some(id)
}

/// Which terminal transition, if any, this contract's own deterministic keys say has
/// already posted (E4b2 verify fold, B1).
///
/// The keys are read **at the record's own generation**, because that is the only
/// generation whose posts belong to the surface the record now carries: an amended
/// contract's generation-0 refund is a different agreement's money.
///
/// A key counts only when it resolves to a transaction the journal actually has and
/// which has not been reversed — the same two questions the `Settled` arm asks, for
/// the same reason.
fn finished_transition(
    c: &EconomicContract,
    snapshot: &EconomySnapshot,
    transactions: &BTreeMap<TransactionId, EconomicTransaction>,
) -> Option<ContractState> {
    for verb in ["settle", "refund", "forfeit"] {
        let key = crate::contract::contract_op_key(c.id, verb, c.amendments);
        let Some(record) = snapshot.idempotency(&key) else { continue };
        if !transactions.contains_key(&record.transaction)
            || snapshot.is_reversed(record.transaction)
        {
            continue;
        }
        return Some(match verb {
            "settle" => ContractState::Settled { transaction: record.transaction },
            "forfeit" => ContractState::Expired,
            _ => ContractState::Cancelled,
        });
    }
    None
}

/// The depositor of every object a live contract's `DepositInto` condition names,
/// taken from the **journal's** last-custody map (§5).
///
/// Only an object the journal last saw going **into** the named vault counts: a
/// withdrawal mark means the thing is not there to give back, and a deposit into a
/// different vault is not this contract's deposit. Bounded by
/// [`crate::contract::MAX_DEPOSITS`] because the module admits one custody op per
/// plan, so a refund could not return more than one anyway.
fn derive_deposits(
    c: &EconomicContract,
    last_custody: &BTreeMap<ObjId, CustodyMark>,
) -> Vec<DepositRecord> {
    let mut out: Vec<DepositRecord> = Vec::new();
    for condition in &c.consent.conditions {
        let TypedCondition::DepositInto { object, vault } = condition else { continue };
        let Some(mark) = last_custody.get(object) else { continue };
        if mark.vault != *vault || mark.direction != CustodyDirection::Deposit {
            continue;
        }
        if out.len() >= crate::contract::MAX_DEPOSITS {
            break;
        }
        out.push(DepositRecord {
            object: *object,
            vault: *vault,
            deposited_by: mark.by.clone(),
            at: mark.at,
        });
    }
    out
}

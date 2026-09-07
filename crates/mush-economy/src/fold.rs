//! **The journal fold** (J3, doc 12 §5.2): the accumulators the boot replay
//! derives from the journal alone — running balances and held amounts per
//! account, the last sequence and the touch count per account, every title's
//! and every reservation's last post-state, the terminal times, the custody
//! marks, the consumed approvals, the reversed set, the allocator high-water
//! marks — as **one value, folded by one function**, so that:
//!
//! - boot and the live store cannot disagree on what the journal says
//!   ([`FoldState::fold_entry`] is what both call, per entry);
//! - the live store carries a **journal-authority shadow** of its projections,
//!   which the checkpoint writer compares against the projections before it
//!   writes (the write-time drift check, doc 12 §5.3) — a runtime drift
//!   detector the boot-only fold never gave;
//! - the checkpoint body is this value at a covered sequence, and a boot from
//!   a checkpoint seeds it and folds only the tail.
//!
//! What is deliberately **not** here: the idempotency index and the
//! transaction refs (O(entries), carried by the checkpoint's immutable parts),
//! the RAM tail (J2), the debit-window tails (live-filtered per D4) and the
//! chunk-to-first-sequence map (a per-chunk fact, not a per-entry one).

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;
use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, ApprovalRequestId, HoldId, TransactionId};
use crate::module::CustodyMark;
use crate::record::{AssetEffectKind, EconomicAsset, EconomicTransaction, Hold, HoldEffectKind};

/// The journal-derived accumulators at [`FoldState::covered_seq`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldState {
    /// The last journal sequence folded, or `None` before any.
    pub covered_seq: Option<u64>,
    /// Per account, the sum of every journal line (`i128`, so an overflow past
    /// `i64` is detected at reconciliation rather than wrapped).
    #[serde(with = "i128_map")]
    pub running: BTreeMap<AccountId, i128>,
    /// Per account, the last sequence carrying a line on it.
    pub last_touch: BTreeMap<AccountId, u64>,
    /// Per account, how many entries carried a line or a hold effect on it
    /// (`1 + touches` is the journal's own revision for the account).
    pub touches: BTreeMap<AccountId, u64>,
    /// Per account, the sum of every reservation still held (placed minus
    /// captured/released/expired).
    #[serde(with = "i128_map")]
    pub running_held: BTreeMap<AccountId, i128>,
    /// Per account, the last sequence carrying a hold effect on it.
    pub last_hold_touch: BTreeMap<AccountId, u64>,
    /// Every title's last post-state (the doc-authority `frozen` latch cleared).
    pub journal_assets: BTreeMap<ObjId, EconomicAsset>,
    /// Every reservation's last post-state.
    pub journal_holds: BTreeMap<HoldId, Hold>,
    /// When each reservation reached a terminal state (for GC).
    pub hold_terminal_at: BTreeMap<HoldId, i64>,
    /// The highest hold id the journal minted.
    pub max_hold_id: Option<u64>,
    /// Every object's last journaled custody movement.
    pub last_custody: BTreeMap<ObjId, CustodyMark>,
    /// The highest vault id a custody effect named.
    pub max_custody_vault: Option<u64>,
    /// Which approval request each consuming transaction spent.
    pub consumed_approval: BTreeMap<ApprovalRequestId, TransactionId>,
    /// The highest approval id the journal consumed.
    pub max_consumed_approval: Option<u64>,
    /// The highest transaction id the journal holds.
    pub max_txid: u64,
    /// Every transaction a later one reversed.
    pub reversed: BTreeSet<TransactionId>,
    /// How many `Created` title effects each object has had (the title
    /// generation).
    pub created_count: BTreeMap<ObjId, u64>,
}

impl FoldState {
    /// Fold one journal entry at `seq`. The one function boot replay and the
    /// live post path both call; a change here changes both.
    pub fn fold_entry(&mut self, seq: u64, tx: &EconomicTransaction) {
        let mut touched: BTreeSet<AccountId> = BTreeSet::new();
        for line in &tx.lines {
            *self.running.entry(line.account).or_insert(0) += line.delta.widen();
            self.last_touch.insert(line.account, seq);
            touched.insert(line.account);
        }
        // E3: the title effects carry the full post-state; the LAST one per
        // object wins. The doc-authority latch is never replayed (D-E3-1).
        for effect in &tx.asset_effects {
            let mut post = effect.post.clone();
            post.last_journal_seq = seq;
            post.frozen = false;
            post.frozen_reason = None;
            self.journal_assets.insert(effect.object, post);
            if effect.kind == AssetEffectKind::Created {
                *self.created_count.entry(effect.object).or_insert(0) += 1;
            }
        }
        // E4b1: each hold effect carries the full post-state; a capture,
        // release or expiry takes the WHOLE reservation off `held`, exactly as
        // `apply` does; a hold effect touches its account like a line (B3).
        for effect in &tx.hold_effects {
            let post = effect.hold.clone();
            touched.insert(post.account);
            self.max_hold_id = Some(self.max_hold_id.map_or(post.id.0, |m| m.max(post.id.0)));
            let signed = if effect.kind == HoldEffectKind::Placed {
                post.amount.widen()
            } else {
                self.hold_terminal_at.insert(post.id, tx.posted_at);
                -post.amount.widen()
            };
            *self.running_held.entry(post.account).or_insert(0) += signed;
            self.last_hold_touch.insert(post.account, seq);
            self.journal_holds.insert(post.id, post);
        }
        for account in touched {
            *self.touches.entry(account).or_insert(0) += 1;
        }
        // E4b2: the depositor is the transaction's actor — journal authority.
        for effect in &tx.custody_effects {
            self.last_custody.insert(
                effect.object,
                CustodyMark {
                    vault: effect.vault,
                    direction: effect.direction,
                    by: tx.actor.clone(),
                    at: tx.posted_at,
                },
            );
            self.max_custody_vault = Some(
                self.max_custody_vault
                    .map_or(effect.vault.0, |m| m.max(effect.vault.0)),
            );
        }
        if let Some(id) = tx.consumed_approval {
            self.consumed_approval.insert(id, tx.id);
            self.max_consumed_approval =
                Some(self.max_consumed_approval.map_or(id.0, |m| m.max(id.0)));
        }
        if let Some(target) = tx.reverses {
            self.reversed.insert(target);
        }
        self.max_txid = self.max_txid.max(tx.id.0);
        self.covered_seq = Some(seq);
    }

    /// Forget a reservation the sweep has garbage-collected (terminal and past
    /// its retention): the shadow stays bounded by live reservations plus the
    /// GC window, exactly like the `hold:` documents.
    pub fn forget_hold(&mut self, id: HoldId) {
        self.journal_holds.remove(&id);
        self.hold_terminal_at.remove(&id);
    }
}

/// `BTreeMap<K, i128>` as JSON — `serde_json` carries no `i128` without a
/// feature, so each value rides as its decimal string.
mod i128_map {
    use std::collections::BTreeMap;

    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<K: Serialize + Ord, S: Serializer>(
        map: &BTreeMap<K, i128>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let as_strings: Vec<(&K, String)> = map.iter().map(|(k, v)| (k, v.to_string())).collect();
        as_strings.serialize(serializer)
    }

    pub fn deserialize<'de, K: Deserialize<'de> + Ord, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<K, i128>, D::Error> {
        let as_strings: Vec<(K, String)> = Vec::deserialize(deserializer)?;
        as_strings
            .into_iter()
            .map(|(k, v)| {
                v.parse::<i128>()
                    .map(|n| (k, n))
                    .map_err(|e| D::Error::custom(format!("not an i128: {v:?}: {e}")))
            })
            .collect()
    }
}

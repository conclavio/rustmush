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
use mush_org::unit_authority::TitleLineage;
use serde::{Deserialize, Serialize};

use crate::ids::{AccountId, ApprovalRequestId, HoldId, TransactionId};
use crate::module::CustodyMark;
use crate::record::{AssetEffectKind, EconomicAsset, EconomicTransaction, Hold, HoldEffectKind};

fn changes_title(kind: AssetEffectKind) -> bool {
    matches!(
        kind,
        AssetEffectKind::Created | AssetEffectKind::TitleTransferred
    )
}

/// Exact bounded title-lineage patch for prepared participant legs that install
/// sparse journal shadows. They share the full fold's event and position rules.
pub(crate) struct TitleLineageDelta {
    changes: BTreeMap<ObjId, (Option<TitleLineage>, TitleLineage)>,
}

impl TitleLineageDelta {
    pub(crate) fn prepare(state: &FoldState, tx: &EconomicTransaction) -> Option<Self> {
        let mut changes = BTreeMap::new();
        for (index, effect) in tx.asset_effects.iter().enumerate() {
            if changes_title(effect.kind) {
                changes.insert(
                    effect.object,
                    (
                        state.title_lineage.get(&effect.object).copied(),
                        TitleLineage {
                            journal_seq: tx.journal_seq,
                            effect_index: u32::try_from(index).ok()?,
                        },
                    ),
                );
            }
        }
        Some(Self { changes })
    }

    pub(crate) fn matches(&self, state: &FoldState) -> bool {
        self.changes
            .iter()
            .all(|(unit, (before, _))| state.title_lineage.get(unit).copied() == *before)
    }

    pub(crate) fn apply(self, state: &mut FoldState) {
        for (unit, (_, lineage)) in self.changes {
            state.title_lineage.insert(unit, lineage);
        }
    }
}

/// The journal-derived accumulators at [`FoldState::covered_seq`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FoldState {
    #[serde(
        default,
        skip_serializing_if = "crate::managed_coverage::book::CoverageBook::uninitialized"
    )]
    pub(crate) coverage_receipts: crate::managed_coverage::book::CoverageBook,
    #[serde(
        default,
        skip_serializing_if = "crate::managed_paid_resale::book::PaidResaleBook::uninitialized"
    )]
    pub(crate) paid_resale_receipts: crate::managed_paid_resale::book::PaidResaleBook,
    #[serde(
        default,
        skip_serializing_if = "crate::managed_charter_transfer::history::CharterPendingAudit::uninitialized"
    )]
    pub(crate) charter_pending_receipts:
        crate::managed_charter_transfer::history::CharterPendingAudit,
    /// Exact charter transfer journal inventory, including its inverse key index.
    #[serde(
        default,
        skip_serializing_if = "crate::registry_filing::charter_transfer::CharterTransferReceiptAudit::uninitialized"
    )]
    pub(crate) charter_transfer_receipts:
        crate::registry_filing::charter_transfer::CharterTransferReceiptAudit,
    #[serde(
        default,
        skip_serializing_if = "crate::core_integration::receipt::CoreReceiptAudit::uninitialized"
    )]
    pub(crate) core_receipts: crate::core_integration::receipt::CoreReceiptAudit,
    #[serde(
        default,
        skip_serializing_if = "crate::core_refit::receipt::RefitReceiptAudit::uninitialized"
    )]
    pub(crate) refit_receipts: crate::core_refit::receipt::RefitReceiptAudit,
    #[serde(
        default,
        skip_serializing_if = "crate::registry_filing::update::UpdateReceiptAudit::uninitialized"
    )]
    pub(crate) update_receipts: crate::registry_filing::update::UpdateReceiptAudit,
    /// Sparse historical proof for registered original filings. Old checkpoints
    /// omit it and remain usable outside commerce; commerce requires full replay
    /// before it can open a missing proof, even when no filings are visible.
    #[serde(
        default,
        skip_serializing_if = "crate::registry_filing::ReceiptAudit::uninitialized"
    )]
    pub(crate) filing_receipts: crate::registry_filing::ReceiptAudit,
    /// Sparse proof of managed original purchases. Its runtime provenance is
    /// opened only by actual replay; old checkpoints omit this exact field.
    #[serde(
        default,
        skip_serializing_if = "crate::managed_purchase::PurchaseReceiptAudit::uninitialized"
    )]
    pub(crate) purchase_receipts: crate::managed_purchase::PurchaseReceiptAudit,
    /// Separate renewal provenance; old checkpoints retain exact omission.
    #[serde(
        default,
        skip_serializing_if = "crate::managed_renewal::receipt::RenewalReceiptAudit::uninitialized"
    )]
    pub(crate) renewal_receipts: crate::managed_renewal::receipt::RenewalReceiptAudit,
    /// Separate renewal provenance; old checkpoints retain exact omission.
    #[serde(
        default,
        skip_serializing_if = "crate::managed_house_renewal::receipt::RenewalReceiptAudit::uninitialized"
    )]
    pub(crate) house_renewal_receipts: crate::managed_house_renewal::receipt::RenewalReceiptAudit,
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
    /// The exact journal effect that most recently created or transferred each
    /// title. Routine asset effects preserve this stamp; even a free transfer
    /// away and back invalidates authority derived from the earlier title.
    ///
    /// Required in checkpoint format 4: an older checkpoint must trigger full
    /// journal replay rather than inventing lineage from an asset revision.
    pub title_lineage: BTreeMap<ObjId, TitleLineage>,
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
        self.paid_resale_receipts.observe(tx);
        self.coverage_receipts.observe(tx);
        self.core_receipts.observe(tx);
        self.refit_receipts.observe(tx);
        self.filing_receipts.observe(tx);
        self.update_receipts.observe(tx);
        self.charter_pending_receipts.observe(tx);
        self.charter_transfer_receipts
            .observe_with_pending(tx, &self.charter_pending_receipts);
        self.purchase_receipts.observe(tx);
        self.renewal_receipts.observe(tx);
        self.house_renewal_receipts.observe(tx);
        let mut touched: BTreeSet<AccountId> = BTreeSet::new();
        for line in &tx.lines {
            *self.running.entry(line.account).or_insert(0) += line.delta.widen();
            self.last_touch.insert(line.account, seq);
            touched.insert(line.account);
        }
        // E3: the title effects carry the full post-state; the LAST one per
        // object wins. The doc-authority latch is never replayed (D-E3-1).
        for (effect_index, effect) in tx.asset_effects.iter().enumerate() {
            let mut post = effect.post.clone();
            post.last_journal_seq = seq;
            post.frozen = false;
            post.frozen_reason = None;
            self.journal_assets.insert(effect.object, post);
            if effect.kind == AssetEffectKind::Created {
                *self.created_count.entry(effect.object).or_insert(0) += 1;
            }
            if changes_title(effect.kind) {
                // Validated posts carry at most MAX_ASSET_OPS_PER_PLAN effects.
                // An unrepresentable imported position must never reuse a prior
                // owner's authority stamp, even though this fold is infallible.
                if let Ok(effect_index) = u32::try_from(effect_index) {
                    self.title_lineage.insert(
                        effect.object,
                        TitleLineage {
                            journal_seq: seq,
                            effect_index,
                        },
                    );
                } else {
                    self.title_lineage.remove(&effect.object);
                }
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

#[cfg(test)]
mod title_lineage_tests {
    use super::*;
    use crate::{AssetEffect, AssetLifecycle, EconomicCause, EconomicVisibility, IdempotencyKey};
    use mush_org::PrincipalId;

    fn asset(object: ObjId, owner: ObjId) -> EconomicAsset {
        EconomicAsset {
            object,
            title_owner: PrincipalId::Object(owner),
            scope: None,
            assignment: None,
            maintenance_account: None,
            catalog: None,
            encumbrances: Vec::new(),
            acquisition: None,
            lifecycle: AssetLifecycle::Active,
            last_journal_seq: 0,
            revision: 1,
            stolen: None,
            frozen: false,
            frozen_reason: None,
        }
    }

    fn entry(effects: Vec<AssetEffect>) -> EconomicTransaction {
        EconomicTransaction {
            id: TransactionId(5),
            // The fold's authoritative sequence argument wins over this field.
            journal_seq: 999,
            idempotency: IdempotencyKey::new("dt-lineage", "fixture"),
            cause: EconomicCause::Staff {
                reason: "fixture".into(),
            },
            actor: PrincipalId::Object(ObjId(1)),
            posted_at: 0,
            description: String::new(),
            lines: Vec::new(),
            reverses: None,
            visibility: EconomicVisibility::Parties,
            plan_hash: [0; 32],
            asset_effects: effects,
            custody_effects: Vec::new(),
            consumed_approval: None,
            hold_effects: Vec::new(),
        }
    }

    fn effect(post: EconomicAsset, kind: AssetEffectKind) -> AssetEffect {
        AssetEffect {
            object: post.object,
            kind,
            post,
        }
    }

    #[test]
    fn dt_title_lineage_tracks_only_title_events_and_exact_effect_positions() {
        let unit = ObjId(10);
        let owner = ObjId(1);
        let mut state = FoldState::default();
        state.fold_entry(
            7,
            &entry(vec![
                effect(asset(ObjId(11), owner), AssetEffectKind::Created),
                effect(asset(unit, owner), AssetEffectKind::Created),
                effect(asset(unit, owner), AssetEffectKind::Assigned),
            ]),
        );
        let initial = TitleLineage {
            journal_seq: 7,
            effect_index: 1,
        };
        assert_eq!(state.title_lineage.get(&unit), Some(&initial));
        for kind in [
            AssetEffectKind::Assigned,
            AssetEffectKind::Unassigned,
            AssetEffectKind::Encumbered,
            AssetEffectKind::Released,
            AssetEffectKind::MaintenanceSet,
            AssetEffectKind::StolenReported,
            AssetEffectKind::CatalogSet,
            AssetEffectKind::Abandoned,
            AssetEffectKind::WrittenOff,
        ] {
            state.fold_entry(8, &entry(vec![effect(asset(unit, owner), kind)]));
            assert_eq!(state.title_lineage.get(&unit), Some(&initial), "{kind:?}");
        }
        // No money/acquisition record changes: away and back in one transaction
        // must still invalidate a grant derived from the original title.
        state.fold_entry(
            9,
            &entry(vec![
                effect(asset(unit, ObjId(2)), AssetEffectKind::TitleTransferred),
                effect(asset(unit, owner), AssetEffectKind::TitleTransferred),
            ]),
        );
        assert_eq!(
            state.journal_assets[&unit].title_owner,
            PrincipalId::Object(owner)
        );
        assert_eq!(
            state.title_lineage[&unit],
            TitleLineage {
                journal_seq: 9,
                effect_index: 1
            }
        );
        assert_eq!(state.created_count[&unit], 1);
        state.fold_entry(
            10,
            &entry(vec![effect(asset(unit, owner), AssetEffectKind::Created)]),
        );
        assert_eq!(
            state.title_lineage[&unit],
            TitleLineage {
                journal_seq: 10,
                effect_index: 0
            }
        );
        assert_eq!(state.created_count[&unit], 2);
    }

    #[test]
    fn dt_title_lineage_round_trips_and_is_required_in_checkpoint_fold() {
        let mut state = FoldState::default();
        state.fold_entry(
            3,
            &entry(vec![effect(
                asset(ObjId(10), ObjId(1)),
                AssetEffectKind::Created,
            )]),
        );
        let value = serde_json::to_value(&state).expect("fold encodes");
        let decoded: FoldState = serde_json::from_value(value.clone()).expect("fold decodes");
        assert_eq!(state, decoded);
        let mut old = value;
        old.as_object_mut()
            .expect("fold object")
            .remove("title_lineage");
        assert!(serde_json::from_value::<FoldState>(old).is_err());
    }

    #[test]
    fn dt_sparse_title_lineage_matches_full_replay_and_refuses_changed_before_image() {
        let unit = ObjId(10);
        let mut initial = FoldState::default();
        initial.fold_entry(
            3,
            &entry(vec![effect(
                asset(unit, ObjId(1)),
                AssetEffectKind::Created,
            )]),
        );
        for kinds in [
            vec![AssetEffectKind::Released],
            vec![AssetEffectKind::Released, AssetEffectKind::TitleTransferred],
            vec![
                AssetEffectKind::TitleTransferred,
                AssetEffectKind::TitleTransferred,
            ],
            vec![AssetEffectKind::Created],
        ] {
            let mut tx = entry(
                kinds
                    .into_iter()
                    .map(|kind| effect(asset(unit, ObjId(2)), kind))
                    .collect(),
            );
            tx.journal_seq = 4;
            let delta = TitleLineageDelta::prepare(&initial, &tx).expect("bounded delta");
            assert!(delta.matches(&initial));
            if tx
                .asset_effects
                .iter()
                .any(|effect| changes_title(effect.kind))
            {
                let mut changed = initial.clone();
                changed.title_lineage.remove(&unit);
                assert!(!delta.matches(&changed));
            }
            let mut sparse = initial.clone();
            delta.apply(&mut sparse);
            let mut full = initial.clone();
            full.fold_entry(tx.journal_seq, &tx);
            assert_eq!(sparse.title_lineage, full.title_lineage);
        }
    }
}

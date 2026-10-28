//! Sparse prebuilt fold delta. At most two monetary lines and one hold;
//! permanent history is never cloned to install one coverage transition.
use super::book::PreparedHistory;
use super::refused;
use crate::AccountId;
use crate::fold::FoldState;
use crate::record::{EconomicAsset, EconomicTransaction, Hold, HoldEffectKind};
use mush_core::participant::ParticipantError;
use std::collections::BTreeSet;

pub(super) struct CoverageFold {
    title_lineage: crate::fold::TitleLineageDelta,
    history: PreparedHistory,
    before_asset: EconomicAsset,
    after_asset: EconomicAsset,
    accounts: Vec<AccountPatch>,
    hold: Option<HoldPatch>,
    before_max_hold: Option<u64>,
    after_max_hold: Option<u64>,
    before_max_tx: u64,
    after_max_tx: u64,
    before_covered: Option<u64>,
    seq: u64,
}
struct AccountPatch {
    id: AccountId,
    before_running: Option<i128>,
    before_last: Option<u64>,
    before_touches: Option<u64>,
    after_running: Option<i128>,
    after_touches: u64,
}
struct HoldPatch {
    before: Option<Hold>,
    before_held: Option<i128>,
    before_last: Option<u64>,
    before_terminal: Option<i64>,
    after: Hold,
    after_held: i128,
    terminal: Option<i64>,
}
impl CoverageFold {
    pub(super) fn prepare(
        state: &FoldState,
        tx: &EconomicTransaction,
    ) -> Result<Self, ParticipantError> {
        if tx.lines.len() > 2
            || tx.hold_effects.len() != 1
            || !(1..=2).contains(&tx.asset_effects.len())
            || !tx.custody_effects.is_empty()
            || tx.consumed_approval.is_some()
            || tx.reverses.is_some()
        {
            return Err(refused());
        }
        let history = state.coverage_receipts.prepare_patch(tx)?;
        let row = history.row();
        let before_asset = state
            .journal_assets
            .get(&row.consent().basis.hull)
            .cloned()
            .ok_or_else(refused)?;
        let expected_before = row
            .terminal()
            .map_or(&row.intent().title, |t| t.before_title());
        if &before_asset != expected_before {
            return Err(refused());
        }
        let after_asset = row
            .terminal()
            .map_or(row.binding().title(), |t| t.title())
            .clone();
        let mut touched: BTreeSet<_> = tx.lines.iter().map(|l| l.account).collect();
        if let Some(effect) = tx.hold_effects.first() {
            touched.insert(effect.hold.account);
        }
        if touched.len() > 2 {
            return Err(refused());
        }
        let mut accounts = Vec::with_capacity(touched.len());
        for id in touched {
            let before_running = state.running.get(&id).copied();
            let before_last = state.last_touch.get(&id).copied();
            let before_touches = state.touches.get(&id).copied();
            let delta = tx
                .lines
                .iter()
                .filter(|l| l.account == id)
                .try_fold(0i128, |n, l| {
                    n.checked_add(l.delta.widen()).ok_or_else(refused)
                })?;
            accounts.push(AccountPatch {
                id,
                before_running,
                before_last,
                before_touches,
                after_running: tx
                    .lines
                    .iter()
                    .any(|l| l.account == id)
                    .then(|| {
                        before_running
                            .unwrap_or(0)
                            .checked_add(delta)
                            .ok_or_else(refused)
                    })
                    .transpose()?,
                after_touches: before_touches
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or_else(refused)?,
            });
        }
        let hold = tx
            .hold_effects
            .first()
            .map(|e| -> Result<HoldPatch, ParticipantError> {
                let before_held = state.running_held.get(&e.hold.account).copied();
                let delta = if e.kind == HoldEffectKind::Placed {
                    e.hold.amount.widen()
                } else {
                    -e.hold.amount.widen()
                };
                let after_held = before_held
                    .unwrap_or(0)
                    .checked_add(delta)
                    .ok_or_else(refused)?;
                if after_held < 0 {
                    return Err(refused());
                }
                Ok(HoldPatch {
                    before: state.journal_holds.get(&e.hold.id).cloned(),
                    before_held,
                    before_last: state.last_hold_touch.get(&e.hold.account).copied(),
                    before_terminal: state.hold_terminal_at.get(&e.hold.id).copied(),
                    after: e.hold.clone(),
                    after_held,
                    terminal: (e.kind != HoldEffectKind::Placed).then_some(tx.posted_at),
                })
            })
            .transpose()?;
        let after_max_hold = hold.as_ref().map_or(state.max_hold_id, |h| {
            Some(
                state
                    .max_hold_id
                    .map_or(h.after.id.0, |old| old.max(h.after.id.0)),
            )
        });
        Ok(Self {
            title_lineage: crate::fold::TitleLineageDelta::prepare(state, tx)
                .ok_or_else(refused)?,
            history,
            before_asset,
            after_asset,
            accounts,
            hold,
            before_max_hold: state.max_hold_id,
            after_max_hold,
            before_max_tx: state.max_txid,
            after_max_tx: state.max_txid.max(tx.id.0),
            before_covered: state.covered_seq,
            seq: tx.journal_seq,
        })
    }
    pub(super) fn matches(&self, state: &FoldState) -> bool {
        self.title_lineage.matches(state)
            && self.history.matches(&state.coverage_receipts)
            && state.journal_assets.get(&self.before_asset.object) == Some(&self.before_asset)
            && state.max_hold_id == self.before_max_hold
            && state.max_txid == self.before_max_tx
            && state.covered_seq == self.before_covered
            && self.accounts.iter().all(|a| {
                state.running.get(&a.id).copied() == a.before_running
                    && state.last_touch.get(&a.id).copied() == a.before_last
                    && state.touches.get(&a.id).copied() == a.before_touches
            })
            && self.hold.as_ref().is_none_or(|h| {
                state.journal_holds.get(&h.after.id) == h.before.as_ref()
                    && state.running_held.get(&h.after.account).copied() == h.before_held
                    && state.last_hold_touch.get(&h.after.account).copied() == h.before_last
                    && state.hold_terminal_at.get(&h.after.id).copied() == h.before_terminal
            })
    }
    pub(super) fn apply(self, state: &mut FoldState) {
        self.title_lineage.apply(state);
        self.history.apply(&mut state.coverage_receipts);
        state
            .journal_assets
            .insert(self.after_asset.object, self.after_asset);
        for a in self.accounts {
            if let Some(n) = a.after_running {
                state.running.insert(a.id, n);
                state.last_touch.insert(a.id, self.seq);
            }
            state.touches.insert(a.id, a.after_touches);
        }
        if let Some(h) = self.hold {
            state.running_held.insert(h.after.account, h.after_held);
            state.last_hold_touch.insert(h.after.account, self.seq);
            if let Some(at) = h.terminal {
                state.hold_terminal_at.insert(h.after.id, at);
            }
            state.journal_holds.insert(h.after.id, h.after);
        }
        state.max_hold_id = self.after_max_hold;
        state.max_txid = self.after_max_tx;
        state.covered_seq = Some(self.seq);
    }
}

//! Exact sparse native bay fee fold delta; this purpose never mutates assets.
use super::refused;
use crate::fold::FoldState;
use crate::record::HoldEffectKind;
use crate::{AccountId, EconomicTransaction, Hold};
use mush_core::participant::ParticipantError;
use std::collections::BTreeSet;
/// Only affected fold entries are copied. A large retained journal is never
/// cloned merely to preflight one bounded purchase.
pub(super) struct BayMoneyFold {
    before: FoldBefore,
    history: super::receipt::PreparedHistory,
    accounts: Vec<FoldAccount>,
    hold: Hold,
    terminal_at: Option<i64>,
    after_held: i128,
    max_hold_id: Option<u64>,
    max_txid: u64,
    covered: Option<u64>,
}
struct FoldBefore {
    accounts: Vec<crate::AccountBeforeImage>,
    held: Option<i128>,
    last_hold_touch: Option<u64>,
    hold: Option<Hold>,
    terminal_at: Option<i64>,
    max_hold_id: Option<u64>,
    max_txid: u64,
    covered: Option<u64>,
}
struct FoldAccount {
    id: AccountId,
    running: Option<i128>,
    touches: u64,
}
impl BayMoneyFold {
    pub(super) fn prepare(
        state: &FoldState,
        tx: &EconomicTransaction,
    ) -> Result<Self, ParticipantError> {
        let [effect] = tx.hold_effects.as_slice() else {
            return Err(refused());
        };
        if tx.lines.len() > 2
            || !tx.asset_effects.is_empty()
            || !tx.custody_effects.is_empty()
            || tx.consumed_approval.is_some()
            || tx.reverses.is_some()
        {
            return Err(refused());
        }
        let mut touched = BTreeSet::from([effect.hold.account]);
        for line in &tx.lines {
            if !touched.insert(line.account) && line.account != effect.hold.account {
                return Err(refused());
            }
        }
        let mut before_accounts = Vec::with_capacity(touched.len());
        let mut accounts = Vec::with_capacity(touched.len());
        for id in touched {
            let running = state.running.get(&id).copied();
            let last = state.last_touch.get(&id).copied();
            let touches = state.touches.get(&id).copied();
            before_accounts.push((id, running, last, touches));
            let delta = tx
                .lines
                .iter()
                .filter(|line| line.account == id)
                .try_fold(0_i128, |total, line| {
                    total.checked_add(line.delta.widen()).ok_or_else(refused)
                })?;
            accounts.push(FoldAccount {
                id,
                running: tx
                    .lines
                    .iter()
                    .any(|line| line.account == id)
                    .then(|| running.unwrap_or(0).checked_add(delta).ok_or_else(refused))
                    .transpose()?,
                touches: touches.unwrap_or(0).checked_add(1).ok_or_else(refused)?,
            });
        }
        let before = FoldBefore {
            accounts: before_accounts,
            held: state.running_held.get(&effect.hold.account).copied(),
            last_hold_touch: state.last_hold_touch.get(&effect.hold.account).copied(),
            hold: state.journal_holds.get(&effect.hold.id).cloned(),
            terminal_at: state.hold_terminal_at.get(&effect.hold.id).copied(),
            max_hold_id: state.max_hold_id,
            max_txid: state.max_txid,
            covered: state.covered_seq,
        };
        // Arithmetic is checked here, before any journal/image is installed.
        let signed = if effect.kind == HoldEffectKind::Placed {
            effect.hold.amount.widen()
        } else {
            -effect.hold.amount.widen()
        };
        let after_held = before
            .held
            .unwrap_or(0)
            .checked_add(signed)
            .ok_or_else(refused)?;
        Ok(Self {
            history: state
                .bay_refit_receipts
                .prepare(tx)
                .map_err(|_| refused())?,
            max_hold_id: Some(
                state
                    .max_hold_id
                    .map_or(effect.hold.id.0, |old| old.max(effect.hold.id.0)),
            ),
            max_txid: state.max_txid.max(tx.id.0),
            covered: Some(tx.journal_seq),
            terminal_at: (effect.kind != HoldEffectKind::Placed).then_some(tx.posted_at),
            after_held,
            hold: effect.hold.clone(),
            accounts,
            before,
        })
    }
    pub(super) fn matches(&self, state: &FoldState) -> bool {
        self.history.matches(&state.bay_refit_receipts)
            && self.before.max_hold_id == state.max_hold_id
            && self.before.max_txid == state.max_txid
            && self.before.covered == state.covered_seq
            && state.running_held.get(&self.hold.account).copied() == self.before.held
            && state.last_hold_touch.get(&self.hold.account).copied() == self.before.last_hold_touch
            && state.journal_holds.get(&self.hold.id) == self.before.hold.as_ref()
            && state.hold_terminal_at.get(&self.hold.id).copied() == self.before.terminal_at
            && self
                .before
                .accounts
                .iter()
                .all(|(id, running, last, touches)| {
                    state.running.get(id).copied() == *running
                        && state.last_touch.get(id).copied() == *last
                        && state.touches.get(id).copied() == *touches
                })
    }
    pub(super) fn apply(self, state: &mut FoldState) {
        self.history.apply(&mut state.bay_refit_receipts);
        // Store the already checked result, not fresh post-publication arithmetic.
        for account in self.accounts {
            if let Some(running) = account.running {
                state.running.insert(account.id, running);
                state
                    .last_touch
                    .insert(account.id, self.hold.last_journal_seq);
            }
            state.touches.insert(account.id, account.touches);
        }
        state
            .running_held
            .insert(self.hold.account, self.after_held);
        state
            .last_hold_touch
            .insert(self.hold.account, self.hold.last_journal_seq);
        if let Some(at) = self.terminal_at {
            state.hold_terminal_at.insert(self.hold.id, at);
        }
        state.journal_holds.insert(self.hold.id, self.hold);
        state.max_hold_id = self.max_hold_id;
        state.max_txid = self.max_txid;
        state.covered_seq = self.covered;
    }
}

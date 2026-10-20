//! Prebuilt E4 images. All local checks precede consuming bind; apply is infallible.
use super::{
    CoreIntegrationConfig,
    fold::CoreMoneyFold,
    money::{self, MoneyPreparation, Output, Request},
    refused,
};
use crate::bloom::{BoundBloomInsert, PreparedBloomInsert};
use crate::fold::FoldState;
use crate::module::{ColdKey, EconomyStore, JOURNAL_LOG, TxRef, WindowTail};
use crate::record::IdempotencyKey;
use crate::registry_filing::{MetaFence, json_write};
use crate::snapshot::EconomySnapshot;
use crate::{
    Account, AccountId, EconomicAsset, EconomicTransaction, Hold, HoldId, InstrumentDef,
    TransactionId,
};
use mush_core::authoritative::ReplayWrite;
use mush_core::ledger::BoundLedgerAppend;
use mush_core::participant::{BoundParticipant, OwnerPrepareContext, ParticipantError};
use mush_core::{OwnerId, PreparedLedgerAppend};
use std::collections::{BTreeMap, BTreeSet};

pub struct CoreMoneyPatch {
    prepared: Box<MoneyPreparation>,
    before_meta: MetaFence,
    before_posts: u64,
    before_accounts: Vec<Account>,
    before_instruments: Vec<InstrumentDef>,
    before_holds: Vec<(HoldId, Option<Hold>)>,
    before_assets: Vec<(mush_core::ObjId, Option<EconomicAsset>)>,
    before_window: Option<WindowTail>,
    next_window: Option<WindowTail>,
    next_transaction: u64,
    next_posts: u64,
    journal: PreparedLedgerAppend,
    bloom: PreparedBloomInsert,
    chunk: u64,
    fold: Box<CoreMoneyFold>,
}
pub(crate) fn prepare(
    config: &CoreIntegrationConfig,
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    request: Request,
) -> Result<(Output, Box<CoreMoneyPatch>, Vec<ReplayWrite>), ParticipantError> {
    let mut prepared = money::prepare(config, state, ctx, request)?;
    let next_transaction = state.next_transaction.checked_add(1).ok_or_else(refused)?;
    let next_posts = state.posts_since_boot.checked_add(1).ok_or_else(refused)?;
    let fold = Box::new(CoreMoneyFold::prepare(&state.fold, &prepared.transaction)?);
    let journal = state
        .journal
        .prepare_append(
            JOURNAL_LOG,
            serde_json::to_value(&prepared.transaction).map_err(|_| ParticipantError::Encoding)?,
        )
        .map_err(|_| refused())?;
    let mut writes = vec![
        journal
            .authoritative_replay_write(OwnerId::new(crate::module::NS).map_err(|_| refused())?)
            .map_err(|_| ParticipantError::Encoding)?,
    ];
    for a in &prepared.delta.accounts {
        writes.push(json_write(crate::api::account_key(a.id), a)?);
    }
    for h in &prepared.delta.holds {
        writes.push(json_write(crate::api::hold_key(h.id), h)?);
    }
    for a in &prepared.delta.assets {
        writes.push(json_write(crate::api::asset_key(a.object), a)?);
    }
    writes.push(json_write(
        crate::keys::idem_key(&prepared.transaction.idempotency),
        &crate::keys::IdemDoc::of(&prepared.transaction),
    )?);
    let mut meta = state.meta_doc();
    let object = meta.as_object_mut().ok_or(ParticipantError::Encoding)?;
    object.insert("next_transaction_id".into(), next_transaction.into());
    object.insert("next_hold_id".into(), prepared.next_hold.into());
    writes.push(json_write(crate::api::META_KEY.into(), &meta)?);
    let mut ids = BTreeSet::from([prepared.history.subject().payer]);
    ids.extend(prepared.transaction.lines.iter().map(|l| l.account));
    let before_accounts = ids
        .into_iter()
        .map(|id| state.snapshot.account(id).cloned().ok_or_else(refused))
        .collect::<Result<Vec<_>, _>>()?;
    let mut instruments = BTreeSet::from([prepared.history.policy().instrument.clone()]);
    if let Some(live) = &prepared.live {
        instruments.extend(live.instruments.iter().map(|i| i.id.clone()));
    }
    let before_instruments = instruments
        .into_iter()
        .map(|id| state.snapshot.instrument(&id).cloned().ok_or_else(refused))
        .collect::<Result<Vec<_>, _>>()?;
    let before_holds = prepared
        .delta
        .holds
        .iter()
        .map(|h| (h.id, state.snapshot.hold(h.id).cloned()))
        .collect();
    let before_assets = prepared
        .delta
        .assets
        .iter()
        .map(|a| (a.object, state.snapshot.asset(a.object).cloned()))
        .collect();
    let payer = prepared.history.subject().payer;
    let before_window = state.debit_window.get(&payer).cloned();
    let mut window = BTreeMap::new();
    if let Some(v) = &before_window {
        window.insert(payer, v.clone());
    }
    crate::module::note_window_debits(&mut window, &prepared.transaction);
    let next_window = if crate::module::account_windowed(&state.snapshot, payer) {
        window.remove(&payer)
    } else {
        None
    };
    let bloom = state
        .key_filter
        .prepare_insert(&prepared.transaction.idempotency)
        .ok_or_else(refused)?;
    let output = prepared.output.take().ok_or_else(refused)?;
    Ok((
        output,
        Box::new(CoreMoneyPatch {
            prepared,
            before_meta: MetaFence::capture(state),
            before_posts: state.posts_since_boot,
            before_accounts,
            before_instruments,
            before_holds,
            before_assets,
            before_window,
            next_window,
            next_transaction,
            next_posts,
            journal,
            bloom,
            chunk: state.journal.open_chunk(JOURNAL_LOG),
            fold,
        }),
        writes,
    ))
}
impl CoreMoneyPatch {
    pub(crate) fn recheck(
        &self,
        config: &CoreIntegrationConfig,
        state: &EconomyStore,
        ctx: &OwnerPrepareContext<'_>,
    ) -> Result<(), ParticipantError> {
        self.check_local(state)?;
        money::recheck(config, state, ctx, &self.prepared)
    }
    pub(crate) fn check_local(&self, state: &EconomyStore) -> Result<(), ParticipantError> {
        if state.frozen
            || !self.before_meta.matches(state)
            || state.posts_since_boot != self.before_posts
            || state.next_transaction != self.prepared.transaction.id.0
            || state.tx_index.contains_key(&self.prepared.transaction.id)
            || state
                .transactions
                .contains_key(&self.prepared.transaction.id)
            || self
                .before_accounts
                .iter()
                .any(|a| state.snapshot.account(a.id) != Some(a))
            || self
                .before_instruments
                .iter()
                .any(|i| state.snapshot.instrument(&i.id) != Some(i))
            || self
                .before_holds
                .iter()
                .any(|(id, h)| state.snapshot.hold(*id) != h.as_ref())
            || self.before_assets.iter().any(|(id, a)| {
                state.snapshot.asset(*id) != a.as_ref()
                    || (a.is_none() && state.snapshot.managed_titles.contains(*id))
            })
            || state
                .debit_window
                .get(&self.prepared.history.subject().payer)
                != self.before_window.as_ref()
            || !self.fold.matches(&state.fold)
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn bind(
        self: Box<Self>,
        state: &mut EconomyStore,
    ) -> Result<BoundCoreMoney<'_>, ParticipantError> {
        self.check_local(state)?;
        let EconomyStore {
            journal,
            key_filter,
            snapshot,
            next_transaction,
            next_hold,
            posts_since_boot,
            tx_index,
            transactions,
            cold_keys,
            fold,
            chunk_firsts,
            chunk_first_ids,
            debit_window,
            ..
        } = state;
        let payer = self.prepared.history.subject().payer;
        let title_birth = self.prepared.history.managed_title_birth();
        let Self {
            prepared,
            journal: prepared_journal,
            bloom,
            fold: fold_patch,
            next_window,
            next_transaction: after_transaction,
            next_posts,
            chunk,
            ..
        } = *self;
        let journal = journal
            .bind_prepared(prepared_journal)
            .map_err(|_| refused())?;
        let bloom = key_filter.bind_insert(bloom).ok_or_else(refused)?;
        Ok(BoundCoreMoney {
            journal,
            bloom,
            snapshot,
            next_transaction,
            next_hold,
            posts_since_boot,
            tx_index,
            transactions,
            cold_keys,
            fold,
            chunk_firsts,
            chunk_first_ids,
            debit_window,
            payer,
            next_window,
            delta: prepared.delta,
            transaction: prepared.transaction,
            after_transaction,
            after_hold: prepared.next_hold,
            next_posts,
            chunk,
            fold_patch,
            title_birth,
        })
    }
}
pub struct BoundCoreMoney<'a> {
    journal: BoundLedgerAppend<'a>,
    bloom: BoundBloomInsert<'a>,
    snapshot: &'a mut EconomySnapshot,
    next_transaction: &'a mut u64,
    next_hold: &'a mut u64,
    posts_since_boot: &'a mut u64,
    tx_index: &'a mut BTreeMap<TransactionId, TxRef>,
    transactions: &'a mut BTreeMap<TransactionId, EconomicTransaction>,
    cold_keys: &'a mut BTreeMap<IdempotencyKey, ColdKey>,
    fold: &'a mut FoldState,
    chunk_firsts: &'a mut BTreeMap<u64, u64>,
    chunk_first_ids: &'a mut BTreeMap<u64, TransactionId>,
    debit_window: &'a mut BTreeMap<AccountId, WindowTail>,
    payer: AccountId,
    next_window: Option<WindowTail>,
    delta: crate::engine::Delta,
    transaction: EconomicTransaction,
    after_transaction: u64,
    after_hold: u64,
    next_posts: u64,
    chunk: u64,
    fold_patch: Box<CoreMoneyFold>,
    title_birth: Option<(mush_core::ObjId, crate::managed_title::TitleBirth)>,
}
impl BoundParticipant for BoundCoreMoney<'_> {
    fn apply(self) {
        let _persisted = self.journal.apply();
        self.delta.apply_to(self.snapshot);
        if let Some((object, birth)) = self.title_birth {
            self.snapshot.managed_titles.install_prepared(object, birth);
        }
        *self.next_transaction = self.after_transaction;
        *self.next_hold = self.after_hold;
        *self.posts_since_boot = self.next_posts;
        self.chunk_firsts
            .entry(self.chunk)
            .or_insert(self.transaction.journal_seq);
        self.chunk_first_ids
            .entry(self.chunk)
            .or_insert(self.transaction.id);
        self.tx_index.insert(
            self.transaction.id,
            TxRef {
                seq: self.transaction.journal_seq,
                idempotency: self.transaction.idempotency.clone(),
                posted_at: self.transaction.posted_at,
            },
        );
        self.cold_keys.remove(&self.transaction.idempotency);
        self.bloom.apply();
        (*self.fold_patch).apply(self.fold);
        match self.next_window {
            Some(w) => {
                self.debit_window.insert(self.payer, w);
            }
            None => {
                self.debit_window.remove(&self.payer);
            }
        }
        self.transactions
            .insert(self.transaction.id, self.transaction);
    }
}

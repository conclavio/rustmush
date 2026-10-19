//! Complete prebuilt native images. All serialization, journal checks and
//! arithmetic precede consuming bind; apply performs no fallible publication.
use super::history::CharterPendingHistory;
use super::native::{
    self, CharterPendingConfig, CharterPendingOutput, ClosedCharterPendingPlan, Request,
};
use super::refused;
use super::sparse::PendingFold;
use crate::bloom::{BoundBloomInsert, PreparedBloomInsert};
use crate::contract::EconomicContract;
use crate::engine::Delta;
use crate::fold::FoldState;
use crate::module::{ColdKey, EconomyStore, JOURNAL_LOG, TxRef, WindowTail};
use crate::plan::{EconomyPlan, Prepared};
use crate::record::{
    Account, EconomicAsset, EconomicTransaction, Hold, IdempotencyKey, InstrumentDef,
};
use crate::registry_filing::{MetaFence, json_write};
use crate::snapshot::EconomySnapshot;
use crate::{AccountId, TransactionId};
use mush_core::PreparedLedgerAppend;
use mush_core::authoritative::ReplayWrite;
use mush_core::ledger::BoundLedgerAppend;
use mush_core::mechanics::OwnerId;
use mush_core::participant::{
    BoundParticipant, OwnerPreparationBinding, OwnerPrepareContext, ParticipantError,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub struct CharterPendingPatch {
    binding: OwnerPreparationBinding,
    request: Request,
    history: CharterPendingHistory,
    before_history: Option<CharterPendingHistory>,
    before_contract: Option<EconomicContract>,
    before_hold: Option<Hold>,
    before_title: EconomicAsset,
    before_birth: crate::managed_title::TitleBirth,
    before_accounts: Vec<Account>,
    before_instrument: InstrumentDef,
    before_meta: MetaFence,
    before_posts: u64,
    before_window: Option<WindowTail>,
    next_window: Option<WindowTail>,
    plan: EconomyPlan,
    validated: Prepared,
    journal: PreparedLedgerAppend,
    bloom: PreparedBloomInsert,
    delta: Delta,
    transaction: EconomicTransaction,
    contract: EconomicContract,
    next_transaction: u64,
    next_posts: u64,
    next_hold: u64,
    next_contract: u64,
    chunk: u64,
    fold: PendingFold,
}
pub(crate) fn prepare(
    config: &CharterPendingConfig,
    state: &EconomyStore,
    ctx: &OwnerPrepareContext<'_>,
    request: Request,
) -> Result<(CharterPendingOutput, CharterPendingPatch, Vec<ReplayWrite>), ParticipantError> {
    let native::Preparation {
        output,
        binding,
        request,
        before_history,
        before_contract,
        before_hold,
        before_title,
        plan,
        validated,
        contract,
        delta,
        transaction,
        next_hold,
        next_contract,
    } = native::prepare(config, state, ctx, request)?;
    let history = output.history.clone();
    let next_transaction = state.next_transaction.checked_add(1).ok_or_else(refused)?;
    let next_posts = state.posts_since_boot.checked_add(1).ok_or_else(refused)?;
    let fold = PendingFold::prepare(&state.fold, &transaction)?;
    let journal = state
        .journal
        .prepare_append(
            JOURNAL_LOG,
            serde_json::to_value(&transaction).map_err(|_| ParticipantError::Encoding)?,
        )
        .map_err(|_| refused())?;
    let mut writes = vec![
        journal
            .authoritative_replay_write(OwnerId::new(crate::module::NS).map_err(|_| refused())?)
            .map_err(|_| ParticipantError::Encoding)?,
    ];
    for a in &delta.accounts {
        writes.push(json_write(crate::api::account_key(a.id), a)?);
    }
    for h in &delta.holds {
        writes.push(json_write(crate::api::hold_key(h.id), h)?);
    }
    for a in &delta.assets {
        writes.push(json_write(crate::api::asset_key(a.object), a)?);
    }
    writes.push(json_write(
        crate::api::contract_key(contract.id),
        &contract,
    )?);
    writes.push(json_write(
        crate::keys::idem_key(&transaction.idempotency),
        &crate::keys::IdemDoc::of(&transaction),
    )?);
    let mut meta = state.meta_doc();
    let object = meta.as_object_mut().ok_or(ParticipantError::Encoding)?;
    object.insert("next_transaction_id".into(), Value::from(next_transaction));
    object.insert("next_hold_id".into(), Value::from(next_hold));
    object.insert("next_contract_id".into(), Value::from(next_contract));
    writes.push(json_write(crate::api::META_KEY.into(), &meta)?);
    let consent = history.consent();
    let mut ids = BTreeSet::from([consent.payer]);
    if history
        .terminal()
        .is_none_or(|t| t.decision().reason.settles())
    {
        ids.insert(consent.terms.seller_account);
    }
    let before_accounts = ids
        .into_iter()
        .map(|id| state.snapshot.account(id).cloned().ok_or_else(refused))
        .collect::<Result<Vec<_>, _>>()?;
    let before_instrument = state
        .snapshot
        .instrument(&consent.terms.instrument)
        .cloned()
        .ok_or_else(refused)?;
    let before_birth = state
        .snapshot
        .managed_titles
        .charter(consent.terms.charter)
        .cloned()
        .ok_or_else(refused)?;
    let before_window = state.debit_window.get(&consent.payer).cloned();
    let mut windows = BTreeMap::new();
    if let Some(w) = &before_window {
        windows.insert(consent.payer, w.clone());
    }
    crate::module::note_window_debits(&mut windows, &transaction);
    let next_window = if crate::module::account_windowed(&state.snapshot, consent.payer) {
        windows.remove(&consent.payer)
    } else {
        None
    };
    let patch = CharterPendingPatch {
        binding,
        request,
        history,
        before_history,
        before_contract,
        before_hold,
        before_title,
        before_birth,
        before_accounts,
        before_instrument,
        before_meta: MetaFence::capture(state),
        before_posts: state.posts_since_boot,
        before_window,
        next_window,
        plan,
        validated,
        journal,
        bloom: state
            .key_filter
            .prepare_insert(&transaction.idempotency)
            .ok_or_else(refused)?,
        delta,
        transaction,
        contract,
        next_transaction,
        next_posts,
        next_hold,
        next_contract,
        chunk: state.journal.open_chunk(JOURNAL_LOG),
        fold,
    };
    Ok((output, patch, writes))
}
impl CharterPendingPatch {
    pub(crate) fn recheck(
        &self,
        config: &CharterPendingConfig,
        state: &EconomyStore,
        ctx: &OwnerPrepareContext<'_>,
    ) -> Result<(), ParticipantError> {
        if !ctx.matches_preparation(&self.binding) {
            return Err(refused());
        }
        self.check_local(state)?;
        let decision = match &self.request {
            Request::Fund(_) => None,
            Request::Terminal(r) => Some(&r.decision),
        };
        native::current(config, state, ctx, self.history.consent(), decision)?;
        let proof = ClosedCharterPendingPlan::retained(
            self.plan.clone(),
            self.history.intent(),
            self.before_contract.clone(),
            self.before_hold.clone(),
            self.before_title.clone(),
        );
        if native::validate(
            state,
            ctx.world(),
            self.history.intent(),
            decision,
            self.before_hold.as_ref(),
            &proof,
        )? != self.validated
        {
            return Err(refused());
        }
        Ok(())
    }
    fn check_local(&self, state: &EconomyStore) -> Result<(), ParticipantError> {
        let consent = self.history.consent();
        if state.frozen
            || !self.before_meta.matches(state)
            || state.posts_since_boot != self.before_posts
            || state.next_transaction != self.transaction.id.0
            || state.snapshot.contract(self.contract.id) != self.before_contract.as_ref()
            || state.snapshot.hold(self.history.funding().hold()) != self.before_hold.as_ref()
            || state
                .fold
                .charter_pending_receipts
                .find(consent.operation)?
                != self.before_history.as_ref()
            || state.snapshot.asset(consent.terms.charter) != Some(&self.before_title)
            || state.snapshot.managed_titles.charter(consent.terms.charter)
                != Some(&self.before_birth)
            || state.snapshot.instrument(&self.before_instrument.id)
                != Some(&self.before_instrument)
            || self
                .before_accounts
                .iter()
                .any(|a| state.snapshot.account(a.id) != Some(a))
            || state.debit_window.get(&consent.payer) != self.before_window.as_ref()
            || state.tx_index.contains_key(&self.transaction.id)
            || state.transactions.contains_key(&self.transaction.id)
            || !self.fold.matches(&state.fold)
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn bind(
        self,
        state: &mut EconomyStore,
    ) -> Result<BoundCharterPending<'_>, ParticipantError> {
        self.check_local(state)?;
        let EconomyStore {
            journal,
            key_filter,
            snapshot,
            next_transaction,
            next_hold,
            next_contract,
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
        let payer = self.history.consent().payer;
        let Self {
            journal: prepared_journal,
            bloom,
            delta,
            transaction,
            contract,
            next_transaction: after_transaction,
            next_hold: after_hold,
            next_contract: after_contract,
            next_posts,
            chunk,
            fold: fold_patch,
            next_window,
            ..
        } = self;
        let journal = journal
            .bind_prepared(prepared_journal)
            .map_err(|_| refused())?;
        let bloom = key_filter.bind_insert(bloom).ok_or_else(refused)?;
        Ok(BoundCharterPending {
            journal,
            bloom,
            snapshot,
            next_transaction,
            next_hold,
            next_contract,
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
            delta,
            transaction,
            contract,
            after_transaction,
            after_hold,
            after_contract,
            next_posts,
            chunk,
            fold_patch,
        })
    }
}
pub struct BoundCharterPending<'a> {
    journal: BoundLedgerAppend<'a>,
    bloom: BoundBloomInsert<'a>,
    snapshot: &'a mut EconomySnapshot,
    next_transaction: &'a mut u64,
    next_hold: &'a mut u64,
    next_contract: &'a mut u64,
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
    delta: Delta,
    transaction: EconomicTransaction,
    contract: EconomicContract,
    after_transaction: u64,
    after_hold: u64,
    after_contract: u64,
    next_posts: u64,
    chunk: u64,
    fold_patch: PendingFold,
}
impl BoundParticipant for BoundCharterPending<'_> {
    fn apply(self) {
        let _already_persisted_write = self.journal.apply();
        self.delta.apply_to(self.snapshot);
        self.snapshot.put_contract(self.contract);
        *self.next_transaction = self.after_transaction;
        *self.next_hold = self.after_hold;
        *self.next_contract = self.after_contract;
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
        self.fold_patch.apply(self.fold);
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

//! Prebuilt purchase images. This is the same EconomyStore participant leg as
//! filing: it never publishes independently or dispatches legacy observers.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::PreparedLedgerAppend;
use mush_core::authoritative::ReplayWrite;
use mush_core::ledger::BoundLedgerAppend;
use mush_core::mechanics::OwnerId;
use mush_core::participant::BoundParticipant;
use serde_json::Value;

use super::*;
use crate::bloom::{BoundBloomInsert, PreparedBloomInsert};
use crate::fold::FoldState;
use crate::module::{ColdKey, TxRef, WindowTail};
use crate::record::{Account, EconomicAsset, Hold, HoldEffectKind, InstrumentDef};
use crate::registry_filing::{MetaFence, json_write};
use crate::snapshot::EconomySnapshot;

/// Only affected fold entries are copied. A large retained journal is never
/// cloned merely to preflight one bounded purchase.
struct PurchaseFold {
    before: FoldBefore,
    history: Box<super::receipt::PreparedHistory>,
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
impl PurchaseFold {
    fn prepare(
        state: &FoldState,
        tx: &EconomicTransaction,
        maximum_payouts: usize,
    ) -> Result<Self, ParticipantError> {
        let [effect] = tx.hold_effects.as_slice() else {
            return Err(refused());
        };
        if tx.lines.len() > maximum_payouts + 1
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
            history: state.purchase_receipts.prepare(tx)?,
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
    fn matches(&self, state: &FoldState) -> bool {
        self.history.matches(&state.purchase_receipts)
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
    fn apply(self, state: &mut FoldState) {
        self.history.apply(&mut state.purchase_receipts);
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

enum PatchRequest {
    Funding(PurchaseSubject),
    Terminal(SubjectTerminal, PurchaseOutcome),
}
impl PatchRequest {
    fn subject(&self) -> &PurchaseSubject {
        match self {
            Self::Funding(request) => request,
            Self::Terminal(request, _) => &request.purchase,
        }
    }
    fn purchase(&self) -> &super::subject::MonetaryPurchase {
        self.subject().common()
    }
    fn sale(&self) -> bool {
        !matches!(self, Self::Terminal(_, PurchaseOutcome::Refunded))
    }
}

pub struct PurchasePatch {
    clock: ClockRef,
    request: PatchRequest,
    plan: EconomyPlan,
    validated: Prepared,
    before_meta: MetaFence,
    before_posts: u64,
    before_accounts: Vec<Account>,
    before_instrument: InstrumentDef,
    before_titles: Vec<EconomicAsset>,
    before_contract: Option<EconomicContract>,
    before_holds: Vec<(HoldId, Option<Hold>)>,
    before_contract_counts: Option<(usize, usize)>,
    before_window: Option<WindowTail>,
    next_window: Option<WindowTail>,
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
    fold: Box<PurchaseFold>,
}

struct ImageInput {
    request: PatchRequest,
    plan: EconomyPlan,
    validated: Prepared,
    before_contract: Option<EconomicContract>,
    contract: EconomicContract,
    delta: Delta,
    transaction: EconomicTransaction,
    next_hold: u64,
    next_contract: u64,
}

fn open_counts(state: &EconomyStore, buyer: ObjId) -> (usize, usize) {
    let buyer = PrincipalId::Object(buyer);
    state
        .snapshot
        .contracts()
        .filter(|row| !row.state.is_terminal())
        .fold((0, 0), |(issued, party), row| {
            (
                issued + usize::from(row.issuer == buyer),
                party + usize::from(row.consent.parties.contains(&buyer)),
            )
        })
}

pub(crate) fn prepare_images(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: impl Into<PurchaseSubject>,
) -> Result<(FundingOutput, Box<PurchasePatch>, Vec<ReplayWrite>), ParticipantError> {
    finish_images(
        state,
        context.world().world_ic_now().clock,
        prepare_funding(state, context, request)?,
    )
}

pub(crate) fn finish_images(
    state: &EconomyStore,
    clock: ClockRef,
    prepared: FundingPreparation,
) -> Result<(FundingOutput, Box<PurchasePatch>, Vec<ReplayWrite>), ParticipantError> {
    let FundingPreparation {
        output,
        request,
        plan,
        validated,
        contract,
        delta,
        transaction,
        next_hold,
        next_contract,
    } = prepared;
    let (patch, writes) = images(
        state,
        clock,
        ImageInput {
            request: PatchRequest::Funding(request),
            plan,
            validated,
            before_contract: None,
            contract,
            delta,
            transaction,
            next_hold,
            next_contract,
        },
    )?;
    Ok((output, patch, writes))
}

pub(crate) fn prepare_terminal_images(
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: impl Into<SubjectTerminal>,
    outcome: PurchaseOutcome,
) -> Result<(TerminalOutput, Box<PurchasePatch>, Vec<ReplayWrite>), ParticipantError> {
    finish_terminal_images(
        state,
        context.world().world_ic_now().clock,
        terminal::prepare(state, context, request, outcome)?,
    )
}

pub(crate) fn finish_terminal_images(
    state: &EconomyStore,
    clock: ClockRef,
    prepared: Box<terminal::TerminalPreparation>,
) -> Result<(TerminalOutput, Box<PurchasePatch>, Vec<ReplayWrite>), ParticipantError> {
    let terminal::TerminalPreparation {
        output,
        request,
        outcome,
        plan,
        validated,
        before_contract,
        contract,
        delta,
        transaction,
    } = *prepared;
    let (patch, writes) = images(
        state,
        clock,
        ImageInput {
            request: PatchRequest::Terminal(request, outcome),
            plan,
            validated,
            before_contract: Some(before_contract),
            contract,
            delta,
            transaction,
            next_hold: state.next_hold,
            next_contract: state.next_contract,
        },
    )?;
    Ok((output, patch, writes))
}

fn images(
    state: &EconomyStore,
    clock: ClockRef,
    input: ImageInput,
) -> Result<(Box<PurchasePatch>, Vec<ReplayWrite>), ParticipantError> {
    let ImageInput {
        request,
        plan,
        validated,
        before_contract,
        contract,
        delta,
        transaction,
        next_hold,
        next_contract,
    } = input;
    let next_transaction = state.next_transaction.checked_add(1).ok_or_else(refused)?;
    let next_posts = state.posts_since_boot.checked_add(1).ok_or_else(refused)?;
    let fold = Box::new(PurchaseFold::prepare(
        &state.fold,
        &transaction,
        request.subject().maximum_payouts(),
    )?);
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
    for account in &delta.accounts {
        writes.push(json_write(crate::api::account_key(account.id), account)?);
    }
    for hold in &delta.holds {
        writes.push(json_write(crate::api::hold_key(hold.id), hold)?);
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
    let purchase = request.purchase();
    let mut ids = BTreeSet::from([purchase.payer]);
    if request.sale() {
        ids.extend(request.subject().account_ids());
    }
    let before_accounts = ids
        .into_iter()
        .map(|id| state.snapshot.account(id).cloned().ok_or_else(refused))
        .collect::<Result<Vec<_>, _>>()?;
    let before_instrument = state
        .snapshot
        .instrument(&purchase.instrument)
        .cloned()
        .ok_or_else(refused)?;
    let before_titles = if request.sale() {
        request
            .subject()
            .title_ids()
            .into_iter()
            .map(|id| state.snapshot.asset(id).cloned().ok_or_else(refused))
            .collect::<Result<Vec<_>, _>>()?
    } else {
        Vec::new()
    };
    let before_contract_counts =
        matches!(&request, PatchRequest::Funding(_)).then(|| open_counts(state, purchase.buyer));
    let before_holds = delta
        .holds
        .iter()
        .map(|hold| (hold.id, state.snapshot.hold(hold.id).cloned()))
        .collect();
    let before_window = state.debit_window.get(&purchase.payer).cloned();
    let mut window = BTreeMap::new();
    if let Some(before) = &before_window {
        window.insert(purchase.payer, before.clone());
    }
    crate::module::note_window_debits(&mut window, &transaction);
    let next_window = if crate::module::account_windowed(&state.snapshot, purchase.payer) {
        window.remove(&purchase.payer)
    } else {
        None
    };
    let patch = Box::new(PurchasePatch {
        clock,
        request,
        plan,
        validated,
        before_meta: MetaFence::capture(state),
        before_posts: state.posts_since_boot,
        before_accounts,
        before_instrument,
        before_titles,
        before_contract,
        before_holds,
        before_contract_counts,
        before_window,
        next_window,
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
    });
    Ok((patch, writes))
}

impl PurchasePatch {
    pub(crate) fn recheck(
        &self,
        state: &EconomyStore,
        context: &OwnerPrepareContext<'_>,
    ) -> Result<(), ParticipantError> {
        self.check_local(state)?;
        if self.clock != context.world().world_ic_now().clock {
            return Err(refused());
        }
        let (plan, validated, contract) = match &self.request {
            PatchRequest::Funding(request) => {
                let prepared = prepare_funding(state, context, request.clone())?;
                (prepared.plan, prepared.validated, prepared.contract)
            }
            PatchRequest::Terminal(request, outcome) => {
                let prepared = terminal::prepare(state, context, request.clone(), *outcome)?;
                (prepared.plan, prepared.validated, prepared.contract)
            }
        };
        if plan != self.plan
            || validated != self.validated
            || contract.consent_hash != self.contract.consent_hash
            || contract.id != self.contract.id
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn check_local(&self, state: &EconomyStore) -> Result<(), ParticipantError> {
        if state.frozen
            || !self.before_meta.matches(state)
            || state.posts_since_boot != self.before_posts
            || state.next_transaction != self.transaction.id.0
            || state.snapshot.contract(self.contract.id) != self.before_contract.as_ref()
            || self
                .before_holds
                .iter()
                .any(|(id, before)| state.snapshot.hold(*id) != before.as_ref())
            || state.tx_index.contains_key(&self.transaction.id)
            || state.transactions.contains_key(&self.transaction.id)
            || self
                .before_titles
                .iter()
                .any(|title| state.snapshot.asset(title.object) != Some(title))
            || state.snapshot.instrument(&self.before_instrument.id)
                != Some(&self.before_instrument)
            || self
                .before_accounts
                .iter()
                .any(|row| state.snapshot.account(row.id) != Some(row))
            || state.debit_window.get(&self.request.purchase().payer) != self.before_window.as_ref()
            || self
                .before_contract_counts
                .is_some_and(|counts| open_counts(state, self.request.purchase().buyer) != counts)
            || !self.fold.matches(&state.fold)
        {
            return Err(refused());
        }
        Ok(())
    }
    pub(crate) fn bind(
        self: Box<Self>,
        state: &mut EconomyStore,
    ) -> Result<BoundPurchase<'_>, ParticipantError> {
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
        let payer = self.request.purchase().payer;
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
        } = *self;
        let journal = journal
            .bind_prepared(prepared_journal)
            .map_err(|_| refused())?;
        let bloom = key_filter.bind_insert(bloom).ok_or_else(refused)?;
        Ok(BoundPurchase {
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

pub struct BoundPurchase<'a> {
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
    fold_patch: Box<PurchaseFold>,
}
impl BoundParticipant for BoundPurchase<'_> {
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
        (*self.fold_patch).apply(self.fold);
        match self.next_window {
            Some(window) => {
                self.debit_window.insert(self.payer, window);
            }
            None => {
                self.debit_window.remove(&self.payer);
            }
        }
        self.transactions
            .insert(self.transaction.id, self.transaction);
        // No World mutation, observer dispatch, serialization or fallible post.
        // The caller settles core custody only after every owner image applies.
    }
}

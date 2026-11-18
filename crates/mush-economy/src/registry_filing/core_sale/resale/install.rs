//! Complete native money, title and hold images for one governed-core resale; no mutable
//! callback follows custody (the buy-back's install, with up to four lines, up to two titles and
//! the hull's hold vault).
use super::*;

struct ResaleFold {
    title_lineage: crate::fold::TitleLineageDelta,
    before_covered: Option<u64>,
    before_max_txid: u64,
    before_accounts: Vec<crate::AccountBeforeImage>,
    after_accounts: Vec<(AccountId, i128, u64, u64)>,
    covered: Option<u64>,
    max_txid: u64,
    receipt: CoreResaleAuditSubject,
    /// Each title this entry writes: its object, the fold's image before and the post image.
    assets: Vec<(ObjId, Option<EconomicAsset>, EconomicAsset)>,
}
impl ResaleFold {
    fn prepare(state: &FoldState, tx: &EconomicTransaction) -> Result<Self, ParticipantError> {
        let receipt = receipt::subject_of(tx).map_err(|_| refused())?;
        if !state.core_resale_receipts.may_admit(&receipt) {
            return Err(refused());
        }
        let mut before_accounts = Vec::with_capacity(tx.lines.len());
        let mut after_accounts = Vec::with_capacity(tx.lines.len());
        for line in &tx.lines {
            let old = state.running.get(&line.account).copied();
            let last = state.last_touch.get(&line.account).copied();
            let touches = state.touches.get(&line.account).copied();
            before_accounts.push((line.account, old, last, touches));
            after_accounts.push((
                line.account,
                old.unwrap_or(0)
                    .checked_add(line.delta.widen())
                    .ok_or_else(refused)?,
                tx.journal_seq,
                touches.unwrap_or(0).checked_add(1).ok_or_else(refused)?,
            ));
        }
        let assets = tx
            .asset_effects
            .iter()
            .map(|effect| {
                (
                    effect.object,
                    state.journal_assets.get(&effect.object).cloned(),
                    effect.post.clone(),
                )
            })
            .collect();
        Ok(Self {
            title_lineage: crate::fold::TitleLineageDelta::prepare(state, tx)
                .ok_or_else(refused)?,
            before_covered: state.covered_seq,
            before_max_txid: state.max_txid,
            before_accounts,
            after_accounts,
            covered: Some(tx.journal_seq),
            max_txid: state.max_txid.max(tx.id.0),
            receipt,
            assets,
        })
    }
    fn matches(&self, state: &FoldState) -> bool {
        self.title_lineage.matches(state)
            && self
                .assets
                .iter()
                .all(|(object, before, _)| state.journal_assets.get(object) == before.as_ref())
            && self.before_covered == state.covered_seq
            && self.before_max_txid == state.max_txid
            && state.core_resale_receipts.may_admit(&self.receipt)
            && self
                .before_accounts
                .iter()
                .all(|(id, running, last, touches)| {
                    state.running.get(id).copied() == *running
                        && state.last_touch.get(id).copied() == *last
                        && state.touches.get(id).copied() == *touches
                })
    }
    fn apply(self, state: &mut FoldState) {
        self.title_lineage.apply(state);
        for (id, running, seq, touches) in self.after_accounts {
            state.running.insert(id, running);
            state.last_touch.insert(id, seq);
            state.touches.insert(id, touches);
        }
        state.max_txid = self.max_txid;
        state.covered_seq = self.covered;
        for (object, _, post) in self.assets {
            state.journal_assets.insert(object, post);
        }
        state.core_resale_receipts.install(self.receipt);
    }
}

pub struct CoreResalePatch {
    binding: mush_core::participant::OwnerPreparationBinding,
    action: DefinitionRef,
    actor: ObjId,
    commit: CommitToken,
    clock: mush_core::ClockRef,
    request: RegistryCoreResale,
    before_titles: Vec<EconomicAsset>,
    before_birth: crate::managed_title::TitleBirth,
    /// The hull's hold, before and after its re-home to the buyer.
    hold: Option<(Vault, Vault)>,
    plan: EconomyPlan,
    validated: Prepared,
    before_meta: MetaFence,
    before_posts: u64,
    before_accounts: Vec<Account>,
    before_instruments: Vec<InstrumentDef>,
    before_window: Option<WindowTail>,
    next_window: Option<WindowTail>,
    payer: AccountId,
    journal: PreparedLedgerAppend,
    bloom: PreparedBloomInsert,
    delta: Delta,
    transaction: EconomicTransaction,
    next_transaction: u64,
    next_posts: u64,
    chunk: u64,
    fold: Box<ResaleFold>,
}

pub(crate) fn prepare(
    config: &CoreResaleConfig,
    policy: &InitialFilingPolicy,
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: RegistryCoreResale,
) -> Result<(CoreResaleOutput, CoreResalePatch, Vec<ReplayWrite>), ParticipantError> {
    require_action(config, context)?;
    let world = context.world();
    let buyer = context.invocation().actor();
    let commit = context.commit_token();
    let (core, hull) = titles(config, policy, state, world, buyer, &request)?;
    let mut before_titles = vec![core.clone()];
    let mut hold = None;
    if let Some((title, vault)) = hull {
        before_titles.push(title.clone());
        if let Some(vault) = vault {
            let after = Vault {
                owner: PrincipalId::Object(buyer),
                scope: None,
                revision: vault.revision.checked_add(1).ok_or_else(refused)?,
                ..vault.clone()
            };
            hold = Some((vault.clone(), after));
        }
    }
    let before_birth = state
        .snapshot
        .managed_titles
        .core(request.core)
        .cloned()
        .ok_or_else(refused)?;
    let plan = plan(
        config,
        policy,
        (buyer, commit, world.world_ic_now().at),
        &request,
    );
    let validated = validate(config, policy, state, world, buyer, &request, &plan)?;
    let id = TransactionId(state.next_transaction);
    let next_transaction = state.next_transaction.checked_add(1).ok_or_else(refused)?;
    let next_posts = state.posts_since_boot.checked_add(1).ok_or_else(refused)?;
    let seq = state.journal.next_seq(JOURNAL_LOG);
    let (delta, transaction) =
        crate::engine::apply_delta(&state.snapshot, &validated, id, seq, world.now());
    let lines = lines(config, &request);
    if !delta.holds.is_empty()
        || delta.consumed_approval.is_some()
        || delta.reversed.is_some()
        || !transaction.custody_effects.is_empty()
        || transaction.lines.len() != lines.len()
        || transaction.asset_effects.len() != before_titles.len()
        || delta.assets.len() != before_titles.len()
    {
        return Err(refused());
    }
    let fold = Box::new(ResaleFold::prepare(&state.fold, &transaction)?);
    let receipt = &fold.receipt;
    if delta.assets.iter().find(|a| a.object == request.core) != Some(&receipt.title)
        || receipt.hull_title.as_ref().is_some_and(|title| {
            delta.assets.iter().find(|a| a.object == title.object) != Some(title)
        })
        || receipt.buyer != buyer
        || receipt.seller != request.seller
        || receipt.core != request.core
        || receipt.hull != request.hull.as_ref().map(|leg| leg.hull)
        || receipt.hold != request.hull.as_ref().and_then(|leg| leg.hold)
        || receipt.hold != hold.as_ref().map(|(before, _)| before.id)
        || receipt.buyer_account != request.buyer_account
        || receipt.seller_account != request.seller_account
        || receipt.terms() != receipt::Terms::of(config)
        || receipt.price != request.price
    {
        return Err(refused());
    }
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
        writes.push(json_write(account_key(account.id), account)?);
    }
    for asset in &delta.assets {
        writes.push(json_write(asset_key(asset.object), asset)?);
    }
    if let Some((_, after)) = &hold {
        writes.push(json_write(crate::api::vault_key(after.id), after)?);
    }
    let key_doc = crate::keys::IdemDoc::of(&transaction);
    writes.push(json_write(
        crate::keys::idem_key(&transaction.idempotency),
        &key_doc,
    )?);
    let before_meta = MetaFence::capture(state);
    let mut next_meta = state.meta_doc();
    let meta = next_meta
        .as_object_mut()
        .ok_or(ParticipantError::Encoding)?;
    meta.insert("next_transaction_id".into(), Value::from(next_transaction));
    writes.push(json_write(crate::api::META_KEY.into(), &next_meta)?);
    let payer = request.buyer_account;
    let mut before_accounts = Vec::with_capacity(lines.len());
    let mut before_instruments = Vec::with_capacity(lines.len());
    for line in &lines {
        let account = state
            .snapshot
            .account(line.account)
            .cloned()
            .ok_or_else(refused)?;
        let instrument = state
            .snapshot
            .instrument(&account.instrument)
            .cloned()
            .ok_or_else(refused)?;
        before_accounts.push(account);
        before_instruments.push(instrument);
    }
    let before_window = state.debit_window.get(&payer).cloned();
    let mut windows = BTreeMap::new();
    if let Some(window) = &before_window {
        windows.insert(payer, window.clone());
    }
    crate::module::note_window_debits(&mut windows, &transaction);
    let next_window = if crate::module::account_windowed(&state.snapshot, payer) {
        windows.remove(&payer)
    } else {
        None
    };
    let bloom = state
        .key_filter
        .prepare_insert(&transaction.idempotency)
        .ok_or_else(refused)?;
    let output = CoreResaleOutput {
        action: config.action.clone(),
        subject: Box::new(fold.receipt.clone()),
    };
    let patch = CoreResalePatch {
        binding: context.preparation_binding(),
        action: config.action.clone(),
        actor: buyer,
        commit,
        clock: world.world_ic_now().clock,
        request,
        before_titles,
        before_birth,
        hold,
        plan,
        validated,
        before_meta,
        before_posts: state.posts_since_boot,
        before_accounts,
        before_instruments,
        before_window,
        next_window,
        payer,
        journal,
        bloom,
        delta,
        transaction,
        next_transaction,
        next_posts,
        chunk: state.journal.open_chunk(JOURNAL_LOG),
        fold,
    };
    Ok((output, patch, writes))
}

pub(crate) fn recheck(
    config: &CoreResaleConfig,
    policy: &InitialFilingPolicy,
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    patch: &CoreResalePatch,
) -> Result<(), ParticipantError> {
    require_action(config, context)?;
    if !context.matches_preparation(&patch.binding)
        || patch.action != config.action
        || patch.actor != context.invocation().actor()
        || patch.commit != context.commit_token()
        || patch.clock != context.world().world_ic_now().clock
    {
        return Err(refused());
    }
    patch.check_local(state)?;
    if validate(
        config,
        policy,
        state,
        context.world(),
        patch.actor,
        &patch.request,
        &patch.plan,
    )? != patch.validated
    {
        return Err(refused());
    }
    Ok(())
}

pub(crate) fn bind(
    state: &mut EconomyStore,
    patch: CoreResalePatch,
) -> Result<BoundCoreResale<'_>, ParticipantError> {
    patch.check_local(state)?;
    let EconomyStore {
        journal,
        snapshot,
        next_transaction,
        posts_since_boot,
        tx_index,
        transactions,
        cold_keys,
        fold,
        chunk_firsts,
        chunk_first_ids,
        debit_window,
        key_filter,
        ..
    } = state;
    let CoreResalePatch {
        journal: prepared_journal,
        bloom,
        delta,
        transaction,
        next_transaction: after_next,
        next_posts,
        chunk,
        fold: fold_patch,
        next_window,
        payer,
        hold,
        ..
    } = patch;
    let journal = journal
        .bind_prepared(prepared_journal)
        .map_err(|_| refused())?;
    let bloom = key_filter.bind_insert(bloom).ok_or_else(refused)?;
    Ok(BoundCoreResale {
        journal,
        bloom,
        snapshot,
        next_transaction,
        posts_since_boot,
        tx_index,
        transactions,
        cold_keys,
        fold,
        chunk_firsts,
        chunk_first_ids,
        debit_window,
        delta,
        transaction,
        after_next,
        next_posts,
        chunk,
        fold_patch,
        next_window,
        payer,
        hold: hold.map(|(_, after)| after),
    })
}
impl CoreResalePatch {
    fn check_local(&self, state: &EconomyStore) -> Result<(), ParticipantError> {
        if state.frozen
            || state.next_transaction != self.transaction.id.0
            || !self.before_meta.matches(state)
            || state.posts_since_boot != self.before_posts
            || self
                .before_titles
                .iter()
                .any(|t| state.snapshot.asset(t.object) != Some(t))
            || state.snapshot.managed_titles.core(self.request.core) != Some(&self.before_birth)
            || self
                .hold
                .as_ref()
                .is_some_and(|(before, _)| state.snapshot.vault(before.id) != Some(before))
            || state.tx_index.contains_key(&self.transaction.id)
            || state.transactions.contains_key(&self.transaction.id)
            || state.debit_window.get(&self.payer) != self.before_window.as_ref()
            || self
                .before_accounts
                .iter()
                .any(|a| state.snapshot.account(a.id) != Some(a))
            || self
                .before_instruments
                .iter()
                .any(|i| state.snapshot.instrument(&i.id) != Some(i))
            || !self.fold.matches(&state.fold)
        {
            return Err(refused());
        }
        Ok(())
    }
}
pub struct BoundCoreResale<'a> {
    journal: BoundLedgerAppend<'a>,
    bloom: BoundBloomInsert<'a>,
    snapshot: &'a mut EconomySnapshot,
    next_transaction: &'a mut u64,
    posts_since_boot: &'a mut u64,
    tx_index: &'a mut BTreeMap<TransactionId, TxRef>,
    transactions: &'a mut BTreeMap<TransactionId, EconomicTransaction>,
    cold_keys: &'a mut BTreeMap<IdempotencyKey, ColdKey>,
    fold: &'a mut FoldState,
    chunk_firsts: &'a mut BTreeMap<u64, u64>,
    chunk_first_ids: &'a mut BTreeMap<u64, TransactionId>,
    debit_window: &'a mut BTreeMap<AccountId, WindowTail>,
    delta: Delta,
    transaction: EconomicTransaction,
    after_next: u64,
    next_posts: u64,
    chunk: u64,
    fold_patch: Box<ResaleFold>,
    next_window: Option<WindowTail>,
    payer: AccountId,
    hold: Option<Vault>,
}
impl BoundParticipant for BoundCoreResale<'_> {
    fn apply(self) {
        let Self {
            journal,
            bloom,
            snapshot,
            next_transaction,
            posts_since_boot,
            tx_index,
            transactions,
            cold_keys,
            fold,
            chunk_firsts,
            chunk_first_ids,
            debit_window,
            delta,
            transaction,
            after_next,
            next_posts,
            chunk,
            fold_patch,
            next_window,
            payer,
            hold,
        } = self;
        let _already_persisted_write = journal.apply();
        delta.apply_to(snapshot);
        // The hold goes with her (Q-48-52): re-homed through `put_vault`, so the owner index
        // moves with it, as `rehome_vault` does.
        if let Some(vault) = hold {
            snapshot.put_vault(vault);
        }
        *next_transaction = after_next;
        *posts_since_boot = next_posts;
        chunk_firsts.entry(chunk).or_insert(transaction.journal_seq);
        chunk_first_ids.entry(chunk).or_insert(transaction.id);
        tx_index.insert(
            transaction.id,
            TxRef {
                seq: transaction.journal_seq,
                idempotency: transaction.idempotency.clone(),
                posted_at: transaction.posted_at,
            },
        );
        cold_keys.remove(&transaction.idempotency);
        bloom.apply();
        (*fold_patch).apply(fold);
        match next_window {
            Some(window) => {
                debit_window.insert(payer, window);
            }
            None => {
                debit_window.remove(&payer);
            }
        }
        transactions.insert(transaction.id, transaction);
        // The one authoritative union already contains every durable image.
        // Deliberately no legacy EconomyObserver dispatch or Outbox publication.
    }
}

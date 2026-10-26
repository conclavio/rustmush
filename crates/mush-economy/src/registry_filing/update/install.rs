//! Complete prebuilt fee-only images. Bind has no World callback; apply cannot fail.
use super::*;

struct UpdateFold {
    before_covered: Option<u64>,
    before_max_txid: u64,
    before_accounts: Vec<crate::AccountBeforeImage>,
    after_accounts: Vec<(AccountId, i128, u64, u64)>,
    covered: Option<u64>,
    max_txid: u64,
    receipt: UpdateAuditSubject,
}
impl UpdateFold {
    fn prepare(state: &FoldState, tx: &EconomicTransaction) -> Result<Self, ParticipantError> {
        let receipt = receipt::subject_of(tx).map_err(|_| refused())?;
        if !state.update_receipts.may_add(receipt.commit) {
            return Err(refused());
        }
        let mut before_accounts = Vec::with_capacity(2);
        let mut after_accounts = Vec::with_capacity(2);
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
        Ok(Self {
            before_covered: state.covered_seq,
            before_max_txid: state.max_txid,
            before_accounts,
            after_accounts,
            covered: Some(tx.journal_seq),
            max_txid: state.max_txid.max(tx.id.0),
            receipt,
        })
    }
    fn matches(&self, state: &FoldState) -> bool {
        self.before_covered == state.covered_seq
            && self.before_max_txid == state.max_txid
            && state.update_receipts.may_add(self.receipt.commit)
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
        for (id, running, seq, touches) in self.after_accounts {
            state.running.insert(id, running);
            state.last_touch.insert(id, seq);
            state.touches.insert(id, touches);
        }
        state.max_txid = self.max_txid;
        state.covered_seq = self.covered;
        state.update_receipts.install(self.receipt);
    }
}

pub struct CompatibleUpdatePatch {
    action: DefinitionRef,
    actor: ObjId,
    commit: CommitToken,
    clock: mush_core::ClockRef,
    request: RegistryCompatibleUpdate,
    authority: UpdateAuthority,
    before_title: EconomicAsset,
    plan: EconomyPlan,
    validated: Prepared,
    before_meta: MetaFence,
    before_posts: u64,
    before_accounts: Vec<Account>,
    before_instruments: Vec<InstrumentDef>,
    before_window: Option<WindowTail>,
    next_window: Option<WindowTail>,
    journal: PreparedLedgerAppend,
    bloom: PreparedBloomInsert,
    delta: Delta,
    transaction: EconomicTransaction,
    next_transaction: u64,
    next_posts: u64,
    chunk: u64,
    fold: Box<UpdateFold>,
}

pub(crate) fn prepare(
    policy: &InitialFilingPolicy,
    action: &DefinitionRef,
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: RegistryCompatibleUpdate,
) -> Result<
    (
        CompatibleUpdateOutput,
        CompatibleUpdatePatch,
        Vec<ReplayWrite>,
    ),
    ParticipantError,
> {
    prepare_inner(
        policy,
        action,
        state,
        context,
        request,
        UpdateAuthority::Personal,
    )
}
pub(crate) fn prepare_house(
    policy: &InitialFilingPolicy,
    config: &house::HouseUpdateConfig,
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: RegistryHouseCompatibleUpdate,
) -> Result<
    (
        CompatibleUpdateOutput,
        CompatibleUpdatePatch,
        Vec<ReplayWrite>,
    ),
    ParticipantError,
> {
    let attribution = HouseFilingAttribution::new(
        context.invocation().actor(),
        request.organization,
        request.scope,
    )?;
    prepare_inner(
        policy,
        &config.action,
        state,
        context,
        RegistryCompatibleUpdate {
            charter: request.charter,
            title_revision: request.title_revision,
            payer: request.payer,
        },
        UpdateAuthority::House {
            config: config.clone(),
            attribution,
        },
    )
}
fn prepare_inner(
    policy: &InitialFilingPolicy,
    action: &DefinitionRef,
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: RegistryCompatibleUpdate,
    authority: UpdateAuthority,
) -> Result<
    (
        CompatibleUpdateOutput,
        CompatibleUpdatePatch,
        Vec<ReplayWrite>,
    ),
    ParticipantError,
> {
    require_action(action, context)?;
    let world = context.world();
    let filer = context.invocation().actor();
    let commit = context.commit_token();
    let before_title = title(state, world, filer, &request, policy, &authority)?.clone();
    if !state.fold.update_receipts.may_add(commit) {
        return Err(refused());
    }
    let kind = match action.key().id.as_str() {
        ACTION_ID => None,
        DERIVED_ACTION_ID => Some(UpdateKind::DependentV1),
        MAJOR_ACTION_ID => Some(UpdateKind::OriginalMajorV1),
        HOUSE_ACTION_ID => Some(UpdateKind::HouseCompatibleV1),
        _ => return Err(refused()),
    };
    if (kind == Some(UpdateKind::HouseCompatibleV1)) != authority.house().is_some() {
        return Err(refused());
    }
    let plan = plan_for(kind, policy, filer, commit, &request, authority.house());
    let validated = validate(policy, state, world, filer, &request, &plan, &authority)?;
    let id = TransactionId(state.next_transaction);
    let next_transaction = state.next_transaction.checked_add(1).ok_or_else(refused)?;
    let next_posts = state.posts_since_boot.checked_add(1).ok_or_else(refused)?;
    let seq = state.journal.next_seq(JOURNAL_LOG);
    let (delta, transaction) =
        crate::engine::apply_delta(&state.snapshot, &validated, id, seq, world.now());
    if !delta.holds.is_empty()
        || delta.consumed_approval.is_some()
        || delta.reversed.is_some()
        || !transaction.custody_effects.is_empty()
        || !transaction.asset_effects.is_empty()
        || !delta.assets.is_empty()
    {
        return Err(refused());
    }
    let fold = Box::new(UpdateFold::prepare(&state.fold, &transaction)?);
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
    let mut before_accounts = Vec::with_capacity(2);
    let mut before_instruments = Vec::with_capacity(2);
    for account in [request.payer, policy.sink] {
        let account = state
            .snapshot
            .account(account)
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
    let before_window = state.debit_window.get(&request.payer).cloned();
    let mut windows = BTreeMap::new();
    if let Some(window) = &before_window {
        windows.insert(request.payer, window.clone());
    }
    crate::module::note_window_debits(&mut windows, &transaction);
    let next_window = if crate::module::account_windowed(&state.snapshot, request.payer) {
        windows.remove(&request.payer)
    } else {
        None
    };
    let bloom = state
        .key_filter
        .prepare_insert(&transaction.idempotency)
        .ok_or_else(refused)?;
    let output = CompatibleUpdateOutput {
        action: action.clone(),
        subject: fold.receipt.clone(),
    };
    let patch = CompatibleUpdatePatch {
        action: action.clone(),
        actor: filer,
        commit,
        clock: world.world_ic_now().clock,
        request,
        authority,
        before_title,
        plan,
        validated,
        before_meta,
        before_posts: state.posts_since_boot,
        before_accounts,
        before_instruments,
        before_window,
        next_window,
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
    policy: &InitialFilingPolicy,
    action: &DefinitionRef,
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    patch: &CompatibleUpdatePatch,
) -> Result<(), ParticipantError> {
    require_action(action, context)?;
    if patch.action != *action
        || patch.actor != context.invocation().actor()
        || patch.commit != context.commit_token()
        || patch.clock != context.world().world_ic_now().clock
    {
        return Err(refused());
    }
    patch.check_local(state)?;
    if validate(
        policy,
        state,
        context.world(),
        patch.actor,
        &patch.request,
        &patch.plan,
        &patch.authority,
    )? != patch.validated
    {
        return Err(refused());
    }
    Ok(())
}
pub(crate) fn bind<'a>(
    state: &'a mut EconomyStore,
    patch: CompatibleUpdatePatch,
) -> Result<BoundCompatibleUpdate<'a>, ParticipantError> {
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
    let CompatibleUpdatePatch {
        journal: prepared_journal,
        bloom,
        delta,
        transaction,
        next_transaction: after_next,
        next_posts,
        chunk,
        fold: fold_patch,
        next_window,
        request,
        ..
    } = patch;
    let journal = journal
        .bind_prepared(prepared_journal)
        .map_err(|_| refused())?;
    let bloom = key_filter.bind_insert(bloom).ok_or_else(refused)?;
    Ok(BoundCompatibleUpdate {
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
        payer: request.payer,
    })
}
impl CompatibleUpdatePatch {
    fn check_local(&self, state: &EconomyStore) -> Result<(), ParticipantError> {
        if state.frozen
            || state.next_transaction != self.transaction.id.0
            || !self.before_meta.matches(state)
            || state.posts_since_boot != self.before_posts
            || state.snapshot.asset(self.request.charter) != Some(&self.before_title)
            || state.tx_index.contains_key(&self.transaction.id)
            || state.transactions.contains_key(&self.transaction.id)
            || state.debit_window.get(&self.request.payer) != self.before_window.as_ref()
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
pub struct BoundCompatibleUpdate<'a> {
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
    fold_patch: Box<UpdateFold>,
    next_window: Option<WindowTail>,
    payer: AccountId,
}
impl BoundParticipant for BoundCompatibleUpdate<'_> {
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
        } = self;
        let _already_persisted_write = journal.apply();
        delta.apply_to(snapshot);
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

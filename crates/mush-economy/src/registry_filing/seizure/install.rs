//! Complete native title images for one seize or forfeit; no mutable callback follows
//! custody (the charter transfer's install, with no money leg).
use super::*;

struct SeizureFold {
    title_lineage: crate::fold::TitleLineageDelta,
    before_covered: Option<u64>,
    before_max_txid: u64,
    covered: Option<u64>,
    max_txid: u64,
    receipt: SeizureAuditSubject,
    before_asset: Option<EconomicAsset>,
    before_held: Option<SeizureAuditSubject>,
}
impl SeizureFold {
    fn prepare(state: &FoldState, tx: &EconomicTransaction) -> Result<Self, ParticipantError> {
        let receipt = receipt::subject_of(tx).map_err(|_| refused())?;
        if !state.seizure_receipts.may_admit(&receipt) {
            return Err(refused());
        }
        Ok(Self {
            title_lineage: crate::fold::TitleLineageDelta::prepare(state, tx)
                .ok_or_else(refused)?,
            before_covered: state.covered_seq,
            before_max_txid: state.max_txid,
            covered: Some(tx.journal_seq),
            max_txid: state.max_txid.max(tx.id.0),
            before_asset: state.journal_assets.get(&receipt.core).cloned(),
            before_held: state.seizure_receipts.held(receipt.core).cloned(),
            receipt,
        })
    }
    fn matches(&self, state: &FoldState) -> bool {
        self.title_lineage.matches(state)
            && state.journal_assets.get(&self.receipt.core) == self.before_asset.as_ref()
            && state.seizure_receipts.held(self.receipt.core) == self.before_held.as_ref()
            && self.before_covered == state.covered_seq
            && self.before_max_txid == state.max_txid
            && state.seizure_receipts.may_admit(&self.receipt)
    }
    fn apply(self, state: &mut FoldState) {
        self.title_lineage.apply(state);
        state.max_txid = self.max_txid;
        state.covered_seq = self.covered;
        state
            .journal_assets
            .insert(self.receipt.core, self.receipt.title.clone());
        state.seizure_receipts.install(self.receipt);
    }
}

pub struct CoreSeizurePatch {
    binding: mush_core::participant::OwnerPreparationBinding,
    action: DefinitionRef,
    actor: ObjId,
    commit: CommitToken,
    clock: mush_core::ClockRef,
    request: RegistryCoreSeizure,
    before_title: EconomicAsset,
    before_birth: crate::managed_title::TitleBirth,
    plan: EconomyPlan,
    validated: Prepared,
    before_meta: MetaFence,
    before_posts: u64,
    journal: PreparedLedgerAppend,
    bloom: PreparedBloomInsert,
    delta: Delta,
    transaction: EconomicTransaction,
    next_transaction: u64,
    next_posts: u64,
    chunk: u64,
    fold: Box<SeizureFold>,
}

pub(crate) fn prepare(
    config: &SeizureConfig,
    policy: &InitialFilingPolicy,
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    request: RegistryCoreSeizure,
) -> Result<(CoreSeizureOutput, CoreSeizurePatch, Vec<ReplayWrite>), ParticipantError> {
    require_action(config, &request.step, context)?;
    let world = context.world();
    let operator = context.invocation().actor();
    let commit = context.commit_token();
    let (before_title, shape) = title(config, policy, state, world, &request)?;
    let before_title = before_title.clone();
    let before_birth = state
        .snapshot
        .managed_titles
        .core(request.core)
        .cloned()
        .ok_or_else(refused)?;
    let plan = plan(
        config,
        policy,
        (operator, commit, world.world_ic_now().at),
        &request,
        &shape,
    );
    let validated = validate(config, policy, state, world, &request, &plan)?;
    let id = TransactionId(state.next_transaction);
    let next_transaction = state.next_transaction.checked_add(1).ok_or_else(refused)?;
    let next_posts = state.posts_since_boot.checked_add(1).ok_or_else(refused)?;
    let seq = state.journal.next_seq(JOURNAL_LOG);
    let (delta, transaction) =
        crate::engine::apply_delta(&state.snapshot, &validated, id, seq, world.now());
    if !delta.holds.is_empty()
        || !delta.accounts.is_empty()
        || delta.consumed_approval.is_some()
        || delta.reversed.is_some()
        || !transaction.custody_effects.is_empty()
        || !transaction.lines.is_empty()
        || transaction.asset_effects.len() != plan.asset_ops.len()
        || delta.assets.len() != 1
    {
        return Err(refused());
    }
    let fold = Box::new(SeizureFold::prepare(&state.fold, &transaction)?);
    let expected_step = match (&request.step, &fold.receipt.step) {
        (SeizureStep::Seize(proof), SeizureStepReceipt::Seize { hull, seq, reason }) => {
            proof.hull == *hull && proof.seq == *seq && proof.key == *reason
        }
        (SeizureStep::Forfeit { seizure }, SeizureStepReceipt::Forfeit { seizure: s, .. }) => {
            seizure == s
        }
        _ => false,
    };
    if delta.assets.first() != Some(&fold.receipt.title)
        || fold.receipt.operator != operator
        || fold.receipt.core != request.core
        || fold.receipt.claimant != config.claimant
        || !expected_step
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
    for asset in &delta.assets {
        writes.push(json_write(asset_key(asset.object), asset)?);
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
    let bloom = state
        .key_filter
        .prepare_insert(&transaction.idempotency)
        .ok_or_else(refused)?;
    let action = action_for(config, &request.step).clone();
    let output = CoreSeizureOutput {
        action: action.clone(),
        subject: fold.receipt.clone(),
    };
    let patch = CoreSeizurePatch {
        binding: context.preparation_binding(),
        action,
        actor: operator,
        commit,
        clock: world.world_ic_now().clock,
        request,
        before_title,
        before_birth,
        plan,
        validated,
        before_meta,
        before_posts: state.posts_since_boot,
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
    config: &SeizureConfig,
    policy: &InitialFilingPolicy,
    state: &EconomyStore,
    context: &OwnerPrepareContext<'_>,
    patch: &CoreSeizurePatch,
) -> Result<(), ParticipantError> {
    require_action(config, &patch.request.step, context)?;
    if !context.matches_preparation(&patch.binding)
        || &patch.action != context.action()
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
    patch: CoreSeizurePatch,
) -> Result<BoundCoreSeizure<'_>, ParticipantError> {
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
        key_filter,
        ..
    } = state;
    let CoreSeizurePatch {
        journal: prepared_journal,
        bloom,
        delta,
        transaction,
        next_transaction: after_next,
        next_posts,
        chunk,
        fold: fold_patch,
        ..
    } = patch;
    let journal = journal
        .bind_prepared(prepared_journal)
        .map_err(|_| refused())?;
    let bloom = key_filter.bind_insert(bloom).ok_or_else(refused)?;
    Ok(BoundCoreSeizure {
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
        delta,
        transaction,
        after_next,
        next_posts,
        chunk,
        fold_patch,
    })
}
impl CoreSeizurePatch {
    fn check_local(&self, state: &EconomyStore) -> Result<(), ParticipantError> {
        if state.frozen
            || state.next_transaction != self.transaction.id.0
            || !self.before_meta.matches(state)
            || state.posts_since_boot != self.before_posts
            || state.snapshot.asset(self.request.core) != Some(&self.before_title)
            || state.snapshot.managed_titles.core(self.request.core) != Some(&self.before_birth)
            || state.tx_index.contains_key(&self.transaction.id)
            || state.transactions.contains_key(&self.transaction.id)
            || !self.fold.matches(&state.fold)
        {
            return Err(refused());
        }
        Ok(())
    }
}
pub struct BoundCoreSeizure<'a> {
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
    delta: Delta,
    transaction: EconomicTransaction,
    after_next: u64,
    next_posts: u64,
    chunk: u64,
    fold_patch: Box<SeizureFold>,
}
impl BoundParticipant for BoundCoreSeizure<'_> {
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
            delta,
            transaction,
            after_next,
            next_posts,
            chunk,
            fold_patch,
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
        transactions.insert(transaction.id, transaction);
        // The one authoritative union already contains every durable image.
        // Deliberately no legacy EconomyObserver dispatch or Outbox publication.
    }
}

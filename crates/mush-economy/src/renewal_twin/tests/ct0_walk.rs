//! Doc 46 CT.0: the one renewal walk, run for both issuers.
//!
//! `run` drives the real owner functions to every terminal state and writes every persisted
//! key, value, digest and identity string it meets. Each family's snapshot test calls it
//! at its own issuer (`managed_renewal::tests::ct0_snapshot`,
//! `managed_house_renewal::ct0_snapshot`) and compares the result with its committed file.
use super::super::{
    self as twin, Offer as _, RenewalAttemptV1, RenewalMarkerV1, RenewalOfferOutput,
    RenewalServiceCloseReason, Terms as _, UnfundedCloseReason,
    funding::{self, MoneyOutput, RenewalMoneyRequest, Transition},
    history::{self, OfferRecord},
    install::{self, OfferContext},
    money_install, receipt, service,
};
use super::{Fixture, Rig};
use crate::engine::{NoCustody, NoFacts, ValidateEnv};
use crate::managed_paid_resale::{PaidHolderRefV1, RenewalHolderBasisV1};
use crate::module::JOURNAL_LOG;
use crate::plan::{EconomyPlan, PlanResult, RequestedLine};
use crate::record::{
    CreditPolicy, EconomicCause, EconomicVisibility, IdempotencyKey, NegativePolicy,
};
use crate::{AccountId, Amount, ContractId, EconomySnapshot, HoldId};
use mush_core::authoritative::ReplayWrite;
use mush_core::mechanics::{
    DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder, OwnerId,
};
use mush_core::participant::BoundParticipant;
use mush_core::{CommitToken, DefinitionDigest, DefinitionRef, GOD};
use mush_org::PrincipalId;
use mush_testkit::snapshot::Snapshot;
use std::collections::BTreeMap;

type Outcome<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Prepared<I> = (
    RenewalOfferOutput<I>,
    Box<install::OfferPatch<I>>,
    Vec<ReplayWrite>,
);

fn token(n: u64) -> CommitToken {
    let mut bytes = [0; 16];
    bytes[..8].copy_from_slice(&n.to_be_bytes());
    CommitToken::from_bytes(bytes)
}
fn reference(
    owner: &str,
    kind: &str,
    id: &str,
    payload: serde_json::Value,
) -> Outcome<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        OwnerId::new(owner)?,
        DefinitionDescriptor::new(
            DefinitionKind::new(kind)?,
            DefinitionId::new(id)?,
            1,
            payload,
        )?,
    )?)
}
fn action(id: &str) -> Outcome<DefinitionRef> {
    reference(
        "market",
        "action",
        id,
        serde_json::json!({"format":1,"transition":id}),
    )
}
fn god() -> mush_core::PrincipalId {
    mush_core::PrincipalId::Object { id: GOD }
}
/// The buyer's context, or the service's (GOD) for a service closure.
fn context<'a, I: Rig>(
    r: &'a Fixture<I>,
    principal: &'a mush_core::PrincipalId,
    action: &'a DefinitionRef,
    commit: u64,
) -> OfferContext<'a> {
    OfferContext {
        world: &r.world,
        actor: match principal {
            mush_core::PrincipalId::Object { id } => *id,
            _ => GOD,
        },
        principal,
        action,
        commit: token(commit),
    }
}
fn request<I: Rig>(out: &RenewalOfferOutput<I>) -> RenewalMoneyRequest {
    RenewalMoneyRequest {
        operation: out.operation(),
        contract: out.contract(),
        consent_hash: out.consent_hash(),
    }
}

/// Fixture cash through a real balanced engine post, as the funded tests do:
/// a system source with unbounded credit pays the buyer 1000.
fn cash<I: Rig>(r: &mut Fixture<I>) -> Outcome {
    let mut source = r
        .state
        .snapshot
        .account(r.offer.terms().payer())
        .ok_or("payer")?
        .clone();
    source.id = AccountId(10);
    source.owner =
        PrincipalId::System(mush_org::SystemPrincipalId::new("renewal.fixture").ok_or("system")?);
    source.scope = None;
    source.credit = CreditPolicy::unbounded();
    r.state.snapshot.put_account(source);
    let mut instrument = r
        .state
        .snapshot
        .instrument(r.offer.terms().instrument())
        .ok_or("instrument")?
        .clone();
    instrument.negative_policy = NegativePolicy::SystemOnly;
    r.state.snapshot.put_instrument(instrument);
    let mut plan = EconomyPlan::new(
        IdempotencyKey::new("test.renewal", "cash"),
        PrincipalId::Object(GOD),
        EconomicCause::Ruleset {
            namespace: "test.renewal".into(),
            correlation: "cash".into(),
        },
        vec![
            RequestedLine {
                account: AccountId(10),
                delta: Amount::from_minor(-1000),
                memo: "fixture treasury debit".into(),
            },
            RequestedLine {
                account: r.offer.terms().payer(),
                delta: Amount::from_minor(1000),
                memo: "fixture buyer cash".into(),
            },
        ],
    );
    plan.visibility = EconomicVisibility::Parties;
    let auth = crate::api::WorldAuth { w: &r.world };
    let keys = r.state.key_gate();
    let env = ValidateEnv {
        auth: &auth,
        custody: &NoCustody,
        facts: &NoFacts,
        now: r.world.now(),
        ic_now: r.world.game_now_unix(),
        next_hold_id: r.state.next_hold,
        spent: BTreeMap::new(),
        keys: &keys,
    };
    let PlanResult::Post(prepared) = crate::engine::validate(&plan, &r.state.snapshot, &env) else {
        return Err("fixture cash post".into());
    };
    let txid = crate::TransactionId(r.state.next_transaction);
    let seq = r.state.journal.next_seq(JOURNAL_LOG);
    let (delta, tx) =
        crate::engine::apply_delta(&r.state.snapshot, &prepared, txid, seq, r.world.now());
    let append = r
        .state
        .journal
        .prepare_append(JOURNAL_LOG, serde_json::to_value(&tx)?)?;
    let (actual_seq, _write) = r.state.journal.apply_prepared(append)?;
    delta.apply_to(&mut r.state.snapshot);
    check_eq!(actual_seq, seq);
    r.state.fold.fold_entry(seq, &tx);
    r.state.transactions.insert(txid, tx.clone());
    r.state.tx_index.insert(
        txid,
        crate::module::TxRef {
            seq,
            idempotency: tx.idempotency.clone(),
            posted_at: tx.posted_at,
        },
    );
    r.state.next_transaction = r
        .state
        .next_transaction
        .checked_add(1)
        .ok_or("transaction")?;
    r.state.posts_since_boot = r.state.posts_since_boot.checked_add(1).ok_or("posts")?;
    I::receipts_mut(&mut r.state.fold).finish_replay()?;
    Ok(())
}

struct Fixtures<I: Rig> {
    offers: Vec<I::Offer>,
    markers: Vec<RenewalMarkerV1<I>>,
    records: Vec<OfferRecord<I>>,
    closures: Vec<history::Closure>,
    services: Vec<service::ServiceRecord<I>>,
    funding: Option<twin::RenewalFundingReceiptV1<I>>,
    terminals: Vec<twin::RenewalTerminalReceiptV1<I>>,
    histories: Vec<twin::RenewalHistory<I>>,
}

// Not derived: a derive would ask the issuer's records for a default of their own.
impl<I: Rig> Default for Fixtures<I> {
    fn default() -> Self {
        Self {
            offers: Vec::new(),
            markers: Vec::new(),
            records: Vec::new(),
            closures: Vec::new(),
            services: Vec::new(),
            funding: None,
            terminals: Vec::new(),
            histories: Vec::new(),
        }
    }
}
struct Walk<I: Rig> {
    snap: Snapshot,
    /// The documents one walk's writes left, by key: what a cold boot reads.
    kv: BTreeMap<String, serde_json::Value>,
    fixtures: Fixtures<I>,
}
impl<I: Rig> Walk<I> {
    fn writes(&mut self, writes: &[ReplayWrite]) -> Outcome {
        self.snap.replay_writes(writes)?;
        for write in writes {
            if let ReplayWrite::ModuleJsonSet {
                namespace,
                key,
                value,
            } = write
            {
                check_eq!(namespace.as_str(), "economy");
                self.kv.insert(key.as_str().to_owned(), value.clone());
            }
        }
        Ok(())
    }
    fn output(&mut self, out: &RenewalOfferOutput<I>) -> Outcome {
        let s = &mut self.snap;
        s.line("output operation", &out.operation().to_hex());
        s.wire("output contract", &out.contract())?;
        s.hex("output consent hash", &out.consent_hash());
        s.wire("output at_ic", out.at_ic())?;
        s.wire("output closed", &out.closed())?;
        s.wire("output service closed", &out.service_closed())?;
        s.wire("output attempt", &out.attempt())?;
        s.wire("output offer digest", &out.offer_digest())?;
        s.line("output commit", &out.commit().to_hex());
        s.wire("output action", out.action())?;
        s.line("output posted_at_utc", &out.posted_at_utc().to_string());
        Ok(())
    }
    /// The persisted owner images of one offer after a transition.
    fn images(&mut self, r: &Fixture<I>, operation: CommitToken) -> Outcome {
        let row = I::book(&r.state).get(operation).ok_or("record")?.clone();
        self.snap.wire("record", &row)?;
        self.snap.wire("record offer", &row.offer)?;
        self.snap
            .wire("record offer digest", &row.offer.digest()?)?;
        self.snap.hex("record consent hash", &row.consent_hash);
        self.snap.line("record key", &history::key::<I>(operation));
        let marker = RenewalMarkerV1::<I>::new(row.operation, row.action.clone(), &row.offer)?;
        self.snap.wire("consent marker", &marker)?;
        let contract = r.state.snapshot.contract(row.contract).ok_or("contract")?;
        self.snap.wire("contract", contract)?;
        self.snap
            .hex("contract consent hash", &contract.consent_hash);
        if let Some(record) = I::book(&r.state).service(operation) {
            self.snap.wire("service record", record)?;
            self.snap.line("service key", &service::key::<I>(operation));
            self.fixtures.services.push(record.clone());
        }
        if let Some(closure) = &row.closure {
            self.fixtures.closures.push(closure.clone());
        }
        self.fixtures.markers.push(marker);
        self.fixtures.records.push(row);
        Ok(())
    }
    fn refusal(
        &mut self,
        label: &str,
        result: Result<Prepared<I>, mush_core::participant::ParticipantError>,
    ) -> Outcome {
        match result {
            Ok(_) => Err(format!("{label}: a repeated transition was accepted").into()),
            Err(error) => {
                self.snap.line(label, &error.to_string());
                Ok(())
            }
        }
    }

    /// Draft (Automatic) at commit 3, at 1000 on the rig's clock.
    fn publish(&mut self, r: &mut Fixture<I>, title: &str) -> Outcome<RenewalOfferOutput<I>> {
        let draft = action(I::DRAFT_ACTION)?;
        let principal = r.principal.clone();
        let prepared = install::offer::<I>(
            &r.state,
            &context(r, &principal, &draft, 3),
            r.offer.clone(),
        )?;
        let (out, writes) = self.install(r, prepared, &principal, &draft, 3)?;
        self.snap.section(&format!("{title} | offered"));
        self.writes(&writes)?;
        self.output(&out)?;
        self.images(r, out.operation())?;
        Ok(out)
    }
    fn install(
        &mut self,
        r: &mut Fixture<I>,
        prepared: Prepared<I>,
        principal: &mush_core::PrincipalId,
        action: &DefinitionRef,
        commit: u64,
    ) -> Outcome<(RenewalOfferOutput<I>, Vec<ReplayWrite>)> {
        let (out, patch, writes) = prepared;
        patch.recheck_in(&r.state, &context(r, principal, action, commit))?;
        patch.bind(&mut r.state)?.apply();
        Ok((out, writes))
    }
    fn close(
        &mut self,
        r: &mut Fixture<I>,
        title: &str,
        operation: u64,
        commit: u64,
        reason: UnfundedCloseReason,
    ) -> Outcome {
        let close = action(I::CLOSE_ACTION)?;
        let principal = r.principal.clone();
        let prepared = install::close::<I>(
            &r.state,
            &context(r, &principal, &close, commit),
            token(operation),
            reason,
        )?;
        let (out, writes) = self.install(r, prepared, &principal, &close, commit)?;
        self.snap.section(&format!("{title} | closed unfunded"));
        self.writes(&writes)?;
        self.output(&out)?;
        self.images(r, token(operation))?;
        let again = install::close::<I>(
            &r.state,
            &context(r, &principal, &close, commit + 100),
            token(operation),
            reason,
        );
        self.refusal("repeated close refused", again)
    }
    fn service_close(
        &mut self,
        r: &mut Fixture<I>,
        title: &str,
        reason: RenewalServiceCloseReason,
    ) -> Outcome {
        let action = action(I::SERVICE_CLOSE_ACTION)?;
        let principal = god();
        let prepared = install::service_close::<I>(
            &r.state,
            &context(r, &principal, &action, 4),
            token(3),
            reason,
        )?;
        let (out, writes) = self.install(r, prepared, &principal, &action, 4)?;
        self.snap
            .section(&format!("{title} | closed by the service"));
        self.writes(&writes)?;
        self.output(&out)?;
        self.images(r, token(3))?;
        let again = install::service_close::<I>(
            &r.state,
            &context(r, &principal, &action, 5),
            token(3),
            reason,
        );
        self.refusal("repeated service close refused", again)
    }
    fn retry(
        &mut self,
        r: &mut Fixture<I>,
        title: &str,
        previous: u64,
        commit: u64,
    ) -> Outcome<RenewalOfferOutput<I>> {
        let draft = action(I::DRAFT_ACTION)?;
        let principal = r.principal.clone();
        let prepared = install::retry::<I>(
            &r.state,
            &context(r, &principal, &draft, commit),
            r.offer.clone(),
            token(previous),
        )?;
        let (out, writes) = self.install(r, prepared, &principal, &draft, commit)?;
        self.snap.section(&format!("{title} | buyer retry offered"));
        self.writes(&writes)?;
        self.output(&out)?;
        self.images(r, out.operation())?;
        Ok(out)
    }
    fn money(
        &mut self,
        r: &mut Fixture<I>,
        title: &str,
        request: RenewalMoneyRequest,
        transition: Transition,
        commit: u64,
    ) -> Outcome {
        let (id, name) = match transition {
            Transition::Fund => (I::FUND_ACTION, "funded"),
            Transition::Grant => (I::GRANT_ACTION, "granted"),
            Transition::Refund => (I::REFUND_ACTION, "refunded"),
        };
        let action = action(id)?;
        let principal = r.principal.clone();
        let operation = request.operation;
        let prepared = funding::prepare_in::<I>(
            &r.state,
            &context(r, &principal, &action, commit),
            request.clone(),
            transition,
        )?;
        let plan_hash = crate::hash::plan_hash(&prepared.plan);
        let (output, patch, writes) =
            money_install::finish_images(&r.state, r.world.world_ic_now().clock, prepared)?;
        patch.check_local(&r.state)?;
        patch.bind(&mut r.state)?.apply();
        self.snap.section(&format!("{title} | {name}"));
        self.writes(&writes)?;
        self.snap.hex("plan hash", &plan_hash);
        match output {
            MoneyOutput::<I>::Funded(out) => {
                check_eq!(out.operation(), operation);
                self.snap.wire("funding receipt", out.receipt())?;
                self.fixtures.funding = Some(out.receipt().clone());
            }
            MoneyOutput::<I>::Terminal(out) => {
                check_eq!(out.operation(), operation);
                self.snap.wire("terminal receipt", out.receipt())?;
                self.fixtures.terminals.push(out.receipt().clone());
            }
        }
        let tx = r.state.transactions.values().last().ok_or("transaction")?;
        self.snap.wire("transaction", tx)?;
        let history = I::receipts(&r.state.fold).get(operation)?;
        self.snap.wire("history", history)?;
        let hold = r
            .state
            .snapshot
            .hold(history.funding().hold())
            .ok_or("hold")?;
        self.snap.wire("hold", hold)?;
        self.fixtures.histories.push(history.clone());
        self.images(r, operation)?;
        let again = funding::prepare_in::<I>(
            &r.state,
            &context(r, &principal, &action, commit + 100),
            request,
            transition,
        );
        match again {
            Ok(_) => Err(format!("{title}: a repeated {name} was accepted").into()),
            Err(error) => {
                self.snap
                    .line("repeated money transition refused", &error.to_string());
                Ok(())
            }
        }
    }

    /// The cold boot: the documents the walk wrote, decoded by the boot
    /// bridge, joined with a fold replayed from the journal. The rebuilt book
    /// must equal the live one; it then replaces it (the combined latch the
    /// service closure needs).
    fn cold(&mut self, r: &mut Fixture<I>, title: &str) -> Outcome {
        let kv: Vec<(smol_str::SmolStr, serde_json::Value)> = self
            .kv
            .iter()
            .map(|(k, v)| (smol_str::SmolStr::new(k), v.clone()))
            .collect();
        let docs = twin::bridge::documents::<I>(&kv, true)?;
        self.snap.section(&format!("{title} | cold rebuild"));
        for (name, bytes) in &docs {
            self.snap.bytes(&format!("document {name}"), bytes);
        }
        let mut fold = crate::fold::FoldState::default();
        let mut entries: Vec<_> = r.state.transactions.values().collect();
        entries.sort_by_key(|tx| tx.journal_seq);
        for tx in entries {
            fold.fold_entry(tx.journal_seq, tx);
        }
        let mut contracts: BTreeMap<ContractId, crate::contract::EconomicContract> = r
            .state
            .snapshot
            .contracts()
            .map(|c| (c.id, c.clone()))
            .collect();
        let holds: BTreeMap<HoldId, crate::record::Hold> = r
            .state
            .snapshot
            .holds()
            .map(|h| (h.id, h.clone()))
            .collect();
        let refused = twin::bridge::recover::<I>(
            &mut crate::fold::FoldState::default(),
            &mut EconomySnapshot::new(),
            &mut contracts.clone(),
            &holds,
            &docs,
            false,
        );
        match refused {
            Ok(_) => return Err("a disabled boot accepted renewal documents".into()),
            Err(error) => self.snap.line("disabled boot refused", &error.to_string()),
        }
        let mut snapshot = EconomySnapshot::new();
        let rebuilt = twin::bridge::recover::<I>(
            &mut fold,
            &mut snapshot,
            &mut contracts,
            &holds,
            &docs,
            true,
        )?;
        check!(rebuilt.combined_verified());
        check_eq!(I::receipts(&fold), I::receipts(&r.state.fold));
        let live = I::book(&r.state);
        check_eq!(rebuilt.operations().len(), live.operations().len());
        let mut operations = Vec::new();
        for operation in live.operations() {
            check_eq!(rebuilt.get(operation), live.get(operation));
            check_eq!(rebuilt.service(operation), live.service(operation));
            let row = live.get(operation).ok_or("row")?;
            check_eq!(
                snapshot.contract(row.contract),
                r.state.snapshot.contract(row.contract)
            );
            operations.push(operation.to_hex());
        }
        self.snap.line("rebuilt operations", &operations.join(","));
        self.snap
            .wire("rebuilt receipt audit", I::receipts(&fold))?;
        *I::book_mut(&mut r.state) = rebuilt;
        Ok(())
    }
}

fn identities<I: Rig>(s: &mut Snapshot) -> Outcome {
    s.section("identities");
    for (label, value) in [
        ("draft action", I::DRAFT_ACTION),
        ("close action", I::CLOSE_ACTION),
        ("fund action", I::FUND_ACTION),
        ("grant action", I::GRANT_ACTION),
        ("refund action", I::REFUND_ACTION),
        ("service close action", I::SERVICE_CLOSE_ACTION),
        ("namespace", I::NAMESPACE),
        ("offer key prefix", I::OFFER_PREFIX),
        ("service key prefix", I::SERVICE_PREFIX),
    ] {
        s.line(label, value);
    }
    s.line("offer key", &history::key::<I>(token(3)));
    s.line("service key", &service::key::<I>(token(3)));
    for (label, value) in [
        (
            "default offer seconds",
            twin::DEFAULT_RENEWAL_OFFER_IC_SECONDS,
        ),
        ("max offer seconds", twin::MAX_RENEWAL_OFFER_IC_SECONDS),
    ] {
        s.line(label, &value.to_string());
    }
    for (label, value) in [
        ("max offer bytes", twin::MAX_RENEWAL_OFFER_BYTES),
        ("max records", history::MAX_RECORDS),
        ("max record bytes", history::MAX_RECORD_BYTES),
        ("max contract bytes", history::MAX_CONTRACT_BYTES),
        ("future reserve bytes", history::FUTURE_RESERVE_BYTES),
        ("max service bytes", service::MAX_BYTES),
        ("max receipt bytes", twin::MAX_RENEWAL_RECEIPT_BYTES),
        ("max history bytes", receipt::MAX_HISTORY_BYTES),
        ("terminal reserve bytes", receipt::TERMINAL_RESERVE_BYTES),
        ("max histories", receipt::MAX_HISTORIES),
        ("max correlation bytes", receipt::MAX_CORRELATION_BYTES),
    ] {
        s.line(label, &value.to_string());
    }
    for (label, error) in [
        ("audit incomplete", twin::RenewalAuditError::Incomplete),
        ("audit refused", twin::RenewalAuditError::Refused),
    ] {
        s.line(label, &error.to_string());
    }
    for reason in [UnfundedCloseReason::Declined, UnfundedCloseReason::Expired] {
        s.wire("unfunded close reason", &reason)?;
    }
    for reason in [
        RenewalServiceCloseReason::DeadlineReached,
        RenewalServiceCloseReason::ClockInvalidated,
    ] {
        s.wire("service close reason", &reason)?;
    }
    for outcome in [
        twin::RenewalOutcome::Granted,
        twin::RenewalOutcome::Refunded,
    ] {
        s.wire("outcome", &outcome)?;
    }
    Ok(())
}

/// The boot configuration's registered actions: built here from the protocol
/// name the twin's bridge must accept, so a moved protocol name, transition
/// tag or reference list fails `validate`; the service action is the bridge's
/// own.
fn configuration<I: Rig>(s: &mut Snapshot) -> Outcome {
    let manifest = reference(
        "tacsim",
        "registry_manifest",
        "combat",
        serde_json::json!({"format":1}),
    )?;
    let economy = reference(
        "economy",
        "owner_participant",
        "initial_filing",
        serde_json::json!({"format":1}),
    )?;
    let roll = reference(
        "roll",
        "owner_participant",
        "commercial_roots",
        serde_json::json!({"format":1}),
    )?;
    let make = |id: &str, transition: &str| -> Outcome<DefinitionRef> {
        Ok(DefinitionRegistryBuilder::new().register(
            OwnerId::new("market")?,
            DefinitionDescriptor::new(
                DefinitionKind::new("action")?,
                DefinitionId::new(id)?,
                1,
                serde_json::json!({"format":1,"protocol":I::PROTOCOL,"transition":transition}),
            )?
            .with_references(vec![manifest.clone(), economy.clone(), roll.clone()]),
        )?)
    };
    let actions = [
        make(I::DRAFT_ACTION, "draft")?,
        make(I::FUND_ACTION, "fund")?,
        make(I::GRANT_ACTION, "grant")?,
        make(I::REFUND_ACTION, "refund")?,
        make(I::CLOSE_ACTION, "unfunded_close")?,
    ];
    let mut configuration = twin::bridge::Configuration::<I> {
        manifest,
        participants: [economy.clone(), roll],
        actions,
        service_close: None,
        issuer: std::marker::PhantomData,
    };
    configuration.validate(&economy)?;
    let service = configuration.service_reference()?;
    configuration.service_close = Some(service.clone());
    configuration.validate(&economy)?;
    s.section("boot configuration");
    s.line("protocol", I::PROTOCOL);
    for (label, reference) in ["draft", "fund", "grant", "refund", "unfunded close"]
        .into_iter()
        .zip(&configuration.actions)
    {
        s.wire(&format!("action {label}"), reference)?;
    }
    s.wire("service close action", &service)?;
    Ok(())
}

/// The retry attempts the record carries. Automatic and BuyerRetry are walked
/// on the real path; the holder attempts need a settled paid resale in the
/// fold (another family's rig), so their record bytes are pinned here from the
/// walked root row with only the attempt changed and the consent recomputed.
fn attempts<I: Rig>(walk: &mut Walk<I>, root: &OfferRecord<I>) -> Outcome {
    let basis = RenewalHolderBasisV1 {
        acceptance: token(40),
        ownership: PaidHolderRefV1 {
            generation: 2,
            operation: token(41),
            digest: DefinitionDigest::from_bytes([97; 32]),
        },
    };
    walk.snap
        .section("attempts | record shapes (holder attempts not walked)");
    let all = [
        RenewalAttemptV1::Automatic,
        RenewalAttemptV1::BuyerRetry {
            previous_operation: token(3),
        },
        RenewalAttemptV1::HolderAutomatic { basis },
        RenewalAttemptV1::HolderRetry {
            basis,
            previous_operation: token(3),
        },
    ];
    for attempt in all {
        walk.snap.wire("attempt", &attempt)?;
        walk.snap
            .line("record format", &attempt.record_format().to_string());
        let mut row = root.clone();
        row.operation = token(50);
        row.closure = None;
        row.format = attempt.record_format();
        row.attempt = (attempt != RenewalAttemptV1::Automatic).then_some(attempt);
        row.consent_hash = twin::contract(&row)?.consent_hash;
        row.validate()?;
        walk.snap.wire("record", &row)?;
        walk.snap.hex("record consent hash", &row.consent_hash);
    }
    walk.snap.codec("RenewalHolderBasisV1", &basis, |json| {
        json["ownership"]["generation"] = 1.into();
    })?;
    // The unit variant has no field list to refuse an unknown field with.
    for attempt in all.into_iter().skip(1) {
        walk.snap.codec_shape("RenewalAttemptV1", &attempt)?;
    }
    Ok(())
}

/// The renewal wire requires an explicit null in every optional economic role
/// (the macro's field attributes on the wire struct): one line a role.
fn required_roles<I: Rig>(s: &mut Snapshot, offer: &I::Offer) -> Outcome {
    for pointer in [
        "/terms/venue",
        "/terms/registry/recipient",
        "/terms/parents/0/title",
    ] {
        let mut json = serde_json::to_value(offer)?;
        let Some((parent, field)) = pointer.rsplit_once('/') else {
            continue;
        };
        let Some(serde_json::Value::Object(object)) = json.pointer_mut(parent) else {
            continue;
        };
        if object.remove(field).is_none() {
            continue;
        }
        match serde_json::from_value::<I::Offer>(json) {
            Ok(_) => s.line(&format!("missing {pointer}"), "accepted"),
            Err(error) => s.line(&format!("missing {pointer} refused"), &error.to_string()),
        }
    }
    Ok(())
}

fn codecs<I: Rig>(walk: &mut Walk<I>) -> Outcome {
    let f = std::mem::take(&mut walk.fixtures);
    let s = &mut walk.snap;
    let fail = |json: &mut serde_json::Value| json["format"] = 2.into();
    for offer in &f.offers {
        s.codec("RenewalBasisV1", offer.basis(), fail)?;
        s.codec("RenewalOfferTermsV1", offer.terms(), fail)?;
        s.codec("RenewalOffer", offer, fail)?;
        required_roles::<I>(s, offer)?;
    }
    for marker in f.markers.iter().take(1) {
        s.codec("RenewalMarkerV1", marker, fail)?;
    }
    for service in &f.services {
        s.codec("RenewalServiceClosureV1", &service.closure, fail)?;
        s.codec_shape("ServiceRecord", service)?;
    }
    if let Some(funding) = &f.funding {
        s.codec("RenewalFundingReceiptV1", funding, fail)?;
    }
    for terminal in &f.terminals {
        s.codec("RenewalTerminalReceiptV1", terminal, fail)?;
    }
    for closure in &f.closures {
        s.codec_shape("Closure", closure)?;
    }
    // One record of each shape: offered and closed, Automatic and retry.
    let mut seen = Vec::new();
    for record in &f.records {
        let shape = (record.format, record.closure.is_some());
        if !seen.contains(&shape) {
            seen.push(shape);
            s.codec_shape("OfferRecord", record)?;
        }
    }
    let mut seen = Vec::new();
    for history in &f.histories {
        let shape = history.terminal().map(|t| t.outcome());
        if !seen.contains(&shape) {
            seen.push(shape);
            s.codec_shape("RenewalHistory", history)?;
        }
    }
    Ok(())
}

/// Every terminal state, each from a fresh rig.
pub(crate) fn run<I: Rig>() -> Outcome<Snapshot> {
    let mut walk = Walk {
        snap: Snapshot::new(),
        kv: BTreeMap::new(),
        fixtures: Fixtures::default(),
    };
    identities::<I>(&mut walk.snap)?;
    configuration::<I>(&mut walk.snap)?;
    {
        let r = Fixture::<I>::new()?;
        walk.snap.section("refusal");
        let empty = install::close::<I>(
            &r.state,
            &context(&r, &r.principal, &action(I::CLOSE_ACTION)?, 4),
            token(3),
            UnfundedCloseReason::Declined,
        );
        walk.refusal("unknown offer refused", empty)?;
    }

    // Declined, then a buyer retry (record format 2) closed Declined again.
    let mut r = Fixture::<I>::new()?;
    walk.kv.clear();
    walk.fixtures.offers.push(r.offer.clone());
    I::receipts_mut(&mut r.state.fold).finish_replay()?;
    walk.publish(&mut r, "declined")?;
    walk.close(&mut r, "declined", 3, 4, UnfundedCloseReason::Declined)?;
    let root = I::book(&r.state).get(token(3)).ok_or("root")?.clone();
    walk.retry(&mut r, "declined", 3, 6)?;
    walk.close(
        &mut r,
        "declined retry",
        6,
        7,
        UnfundedCloseReason::Declined,
    )?;
    walk.cold(&mut r, "declined")?;
    attempts(&mut walk, &root)?;

    // Expired: closed by the buyer at the deadline.
    let mut r = Fixture::<I>::new()?;
    walk.kv.clear();
    I::receipts_mut(&mut r.state.fold).finish_replay()?;
    walk.publish(&mut r, "expired")?;
    let deadline = r.offer.terms().expires_at().at;
    r.world.set_now(deadline);
    walk.close(&mut r, "expired", 3, 4, UnfundedCloseReason::Expired)?;
    walk.cold(&mut r, "expired")?;

    // Closed by the service, each reason, after a cold boot latched the book.
    for reason in [
        RenewalServiceCloseReason::DeadlineReached,
        RenewalServiceCloseReason::ClockInvalidated,
    ] {
        let title = match reason {
            RenewalServiceCloseReason::DeadlineReached => "service deadline",
            RenewalServiceCloseReason::ClockInvalidated => "service clock",
        };
        let mut r = Fixture::<I>::new()?;
        walk.kv.clear();
        I::receipts_mut(&mut r.state.fold).finish_replay()?;
        walk.publish(&mut r, title)?;
        walk.cold(&mut r, &format!("{title} offered"))?;
        match reason {
            RenewalServiceCloseReason::DeadlineReached => {
                let deadline = r.offer.terms().expires_at().at;
                r.world.set_now(deadline);
            }
            RenewalServiceCloseReason::ClockInvalidated => {
                let epoch = r.world.next_world_timeline_epoch(2)?;
                r.world.append_world_timeline_epoch(epoch)?;
            }
        }
        walk.service_close(&mut r, title, reason)?;
        walk.cold(&mut r, title)?;
    }

    // Funded and granted, with every payee role the twin has.
    let mut r = Fixture::<I>::new()?;
    walk.kv.clear();
    I::widen(&mut r)?;
    walk.fixtures.offers.push(r.offer.clone());
    cash(&mut r)?;
    let offered = walk.publish(&mut r, "granted")?;
    walk.money(&mut r, "granted", request(&offered), Transition::Fund, 4)?;
    walk.cold(&mut r, "granted funded")?;
    walk.money(&mut r, "granted", request(&offered), Transition::Grant, 5)?;
    walk.cold(&mut r, "granted")?;

    // Funded and refunded; then a buyer retry funded and granted.
    let mut r = Fixture::<I>::new()?;
    walk.kv.clear();
    cash(&mut r)?;
    let offered = walk.publish(&mut r, "refunded")?;
    walk.money(&mut r, "refunded", request(&offered), Transition::Fund, 4)?;
    walk.money(&mut r, "refunded", request(&offered), Transition::Refund, 5)?;
    walk.cold(&mut r, "refunded")?;
    let retried = walk.retry(&mut r, "refunded", 3, 6)?;
    walk.money(
        &mut r,
        "refunded retry",
        request(&retried),
        Transition::Fund,
        7,
    )?;
    walk.money(
        &mut r,
        "refunded retry",
        request(&retried),
        Transition::Grant,
        8,
    )?;
    walk.cold(&mut r, "refunded retry")?;

    codecs(&mut walk)?;
    Ok(walk.snap)
}

//! Actual pure E4 plans, journal transactions and consuming owner images. These
//! are isolated owner tests, not a paired licence/boot activation claim.
use super::super::{
    funding::{self, MoneyOutput, RenewalMoneyRequest, Transition},
    money_install, receipt,
};
use super::*;
#[path = "funded/bridge.rs"]
mod bridge_tests;
use crate::engine::{NoCustody, NoFacts, ValidateEnv};
use crate::module::JOURNAL_LOG;
use crate::plan::{EconomyPlan, PlanResult, RequestedLine};
use crate::record::{EconomicCause, EconomicVisibility, IdempotencyKey};
use std::collections::BTreeMap;

fn cash(r: &mut Rig) -> TestResult {
    // Lawful fixture system source, then a real balanced engine post. The buyer
    // starts at zero and no posted_balance is edited to manufacture escrow cash.
    let mut source = r
        .state
        .snapshot
        .account(AccountId(0))
        .ok_or("payer")?
        .clone();
    source.id = AccountId(10);
    source.owner =
        PrincipalId::System(mush_org::SystemPrincipalId::new("renewal.fixture").ok_or("system")?);
    source.credit = CreditPolicy::unbounded();
    r.state.snapshot.put_account(source);
    let mut instrument = r
        .state
        .snapshot
        .instrument(&r.offer.terms.instrument)
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
                account: r.offer.terms.payer,
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
        return Err("actual fixture cash post".into());
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
    assert_eq!(actual_seq, seq);
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
    assert_eq!(
        r.state
            .snapshot
            .account(r.offer.terms.payer)
            .ok_or("payer")?
            .posted_balance,
        Amount::from_minor(1000)
    );
    assert_eq!(
        r.state
            .snapshot
            .account(AccountId(10))
            .ok_or("source")?
            .posted_balance,
        Amount::from_minor(-1000)
    );
    // Private actual empty-renewal replay: the sole real journal entry above
    // contains no renewal. This cannot publish production readiness.
    r.state.fold.renewal_receipts.finish_replay()?;
    Ok(())
}
fn request(out: &RenewalOfferOutput) -> RenewalMoneyRequest {
    RenewalMoneyRequest {
        operation: out.operation(),
        contract: out.contract(),
        consent_hash: out.consent_hash(),
    }
}
fn prepared(
    r: &Rig,
    request: RenewalMoneyRequest,
    t: Transition,
    n: u64,
) -> TestResult<Box<funding::MoneyPreparation>> {
    let name = match t {
        Transition::Fund => FUND_ACTION,
        Transition::Grant => GRANT_ACTION,
        Transition::Refund => REFUND_ACTION,
    };
    let action = action(name, 1)?;
    Ok(funding::prepare_in(
        &r.state,
        &OfferContext {
            world: &r.world,
            actor: r.offer.terms.buyer,
            principal: &r.principal,
            action: &action,
            commit: token(n),
        },
        request,
        t,
    )?)
}
fn install_money(
    r: &mut Rig,
    p: Box<funding::MoneyPreparation>,
) -> TestResult<(MoneyOutput, Vec<ReplayWrite>)> {
    let (output, patch, writes) =
        money_install::finish_images(&r.state, r.world.world_ic_now().clock, p)?;
    patch.check_local(&r.state)?;
    patch.bind(&mut r.state)?.apply();
    Ok((output, writes))
}
fn funded(six: bool) -> TestResult<(Box<Rig>, RenewalMoneyRequest, Vec<(String, Vec<u8>)>)> {
    let mut r = Rig::new()?;
    if six {
        r.six_roles()?;
    }
    cash(&mut r)?;
    let (offer, writes) = r.publish()?;
    let req = request(&offer);
    let docs = documents(&writes)?;
    let prep = prepared(&r, req.clone(), Transition::Fund, 4)?;
    install_money(&mut r, prep)?;
    Ok((r, req, docs))
}
#[test]
fn renewal_funding_keeps_exact_contract_consent_and_accepts_one_real_cash_hold() -> TestResult {
    let mut r = Rig::new()?;
    cash(&mut r)?;
    let (offer, writes) = r.publish()?;
    let req = request(&offer);
    let allocator = r.state.next_contract;
    let before = r
        .state
        .snapshot
        .contract(req.contract)
        .ok_or("contract")?
        .clone();
    let book = serde_json::to_vec(r.state.renewal_offers.get(req.operation).ok_or("offer")?)?;
    let prep = prepared(&r, req.clone(), Transition::Fund, 4)?;
    let (_, patch, _) = money_install::finish_images(&r.state, r.world.world_ic_now().clock, prep)?;
    drop(patch);
    assert_eq!(r.state.snapshot.contract(req.contract), Some(&before));
    assert_eq!(
        r.state
            .snapshot
            .account(r.offer.terms.payer)
            .ok_or("payer")?
            .held_amount,
        Amount::ZERO
    );
    let prep = prepared(&r, req.clone(), Transition::Fund, 4)?;
    let (output, images) = install_money(&mut r, prep)?;
    let MoneyOutput::Funded(output) = output else {
        return Err("funded output".into());
    };
    let after = r.state.snapshot.contract(req.contract).ok_or("contract")?;
    assert_eq!(r.state.next_contract, allocator);
    assert_eq!(after.id, before.id);
    assert_eq!(after.consent, before.consent);
    assert_eq!(after.consent_hash, before.consent_hash);
    assert_eq!(after.acceptances.len(), 1);
    assert_eq!(after.escrow.as_slice(), &[output.receipt().hold()]);
    assert_eq!(after.revision, 2);
    assert_eq!(after.state, ContractState::ReadyToSettle);
    assert_eq!(
        serde_json::to_vec(r.state.renewal_offers.get(req.operation).ok_or("offer")?)?,
        book
    );
    assert_eq!(
        r.state
            .snapshot
            .account(r.offer.terms.payer)
            .ok_or("payer")?
            .held_amount,
        r.offer.terms.price
    );
    assert!(prepared(&r, req.clone(), Transition::Fund, 5).is_err());
    let docs = documents(&writes)?;
    OfferBook::from_documents_with_renewals(
        &r.state.snapshot,
        &docs,
        &r.state.fold.renewal_receipts,
    )?;
    assert!(
        OfferBook::from_documents(&r.state.snapshot, &docs).is_err(),
        "old unfunded verifier must not infer funding"
    );
    assert!(!images.is_empty());
    Ok(())
}
#[test]
fn renewal_six_payee_capture_and_refund_have_exact_real_journal_provenance() -> TestResult {
    for transition in [Transition::Grant, Transition::Refund] {
        let (mut r, req, docs) = funded(true)?;
        let allocator = r.state.next_contract;
        let hold_allocator = r.state.next_hold;
        let prep = prepared(&r, req.clone(), transition, 5)?;
        let tx = prep.transaction.clone();
        if transition == Transition::Grant {
            assert_eq!(tx.lines.len(), 7);
        } else {
            assert!(tx.lines.is_empty());
        }
        let (out, _) = install_money(&mut r, prep)?;
        let MoneyOutput::Terminal(out) = out else {
            return Err("terminal".into());
        };
        assert_eq!(out.receipt().commit(), token(5));
        assert_eq!(r.state.next_contract, allocator);
        assert_eq!(r.state.next_hold, hold_allocator);
        assert_eq!(
            r.state
                .snapshot
                .account(r.offer.terms.payer)
                .ok_or("payer")?
                .held_amount,
            Amount::ZERO
        );
        assert_eq!(
            r.state
                .snapshot
                .account(r.offer.terms.payer)
                .ok_or("payer")?
                .posted_balance,
            Amount::from_minor(if transition == Transition::Grant {
                900
            } else {
                1000
            })
        );
        let h = r.state.fold.renewal_receipts.get(req.operation)?;
        assert_eq!(h.terminal(), Some(out.receipt()));
        h.verify_documents(
            r.state.snapshot.contract(req.contract).ok_or("contract")?,
            r.state.snapshot.hold(h.funding().hold()).ok_or("hold")?,
        )?;
        OfferBook::from_documents_with_renewals(
            &r.state.snapshot,
            &docs,
            &r.state.fold.renewal_receipts,
        )?;
        let mut replay = crate::fold::FoldState::default();
        let mut transactions = r.state.transactions.values().collect::<Vec<_>>();
        transactions.sort_by_key(|tx| tx.journal_seq);
        for tx in transactions {
            replay.fold_entry(tx.journal_seq, tx);
        }
        replay.renewal_receipts.finish_replay()?;
        assert_eq!(
            r.state.fold, replay,
            "consuming install must equal actual journal replay"
        );
        assert!(prepared(&r, req, transition, 6).is_err());
    }
    Ok(())
}

#[test]
fn renewal_exhausted_money_allocators_refuse_before_any_image_install() -> TestResult {
    for hold in [false, true] {
        let mut r = Rig::new()?;
        cash(&mut r)?;
        let (offered, _) = r.publish()?;
        if hold {
            r.state.next_hold = u64::MAX;
        } else {
            r.state.next_transaction = u64::MAX;
        }
        let before = r
            .state
            .snapshot
            .contract(offered.contract())
            .ok_or("contract")?
            .clone();
        let result = prepared(&r, request(&offered), Transition::Fund, 4).and_then(|p| {
            Ok(money_install::finish_images(
                &r.state,
                r.world.world_ic_now().clock,
                p,
            )?)
        });
        assert!(result.is_err());
        assert_eq!(r.state.snapshot.holds().count(), 0);
        assert_eq!(r.state.snapshot.contract(offered.contract()), Some(&before));
        assert_eq!(
            r.state
                .snapshot
                .account(r.offer.terms.payer)
                .ok_or("payer")?
                .held_amount,
            Amount::ZERO
        );
    }
    Ok(())
}
#[test]
fn renewal_refund_ignores_seller_lapse_and_old_clock_but_grant_does_not() -> TestResult {
    let (mut r, req, _) = funded(false)?;
    let mut title = r
        .state
        .snapshot
        .asset(r.offer.terms.charter)
        .ok_or("title")?
        .clone();
    title.frozen = true;
    r.state.snapshot.put_asset(title);
    assert!(prepared(&r, req.clone(), Transition::Grant, 5).is_err());
    let epoch = r.world.next_world_timeline_epoch(2)?;
    r.world.append_world_timeline_epoch(epoch)?;
    let prep = prepared(&r, req, Transition::Refund, 5)?;
    let (out, _) = install_money(&mut r, prep)?;
    let MoneyOutput::Terminal(out) = out else {
        return Err("refund".into());
    };
    assert_eq!(out.receipt().issued_at_ic(), None);
    Ok(())
}
#[test]
fn renewal_history_cannot_be_minted_by_mutable_contract_or_decoded_checkpoint() -> TestResult {
    let (r, req, docs) = funded(false)?;
    let raw = serde_json::to_vec(&r.state.fold.renewal_receipts)?;
    let decoded: receipt::RenewalReceiptAudit = serde_json::from_slice(&raw)?;
    assert!(decoded.require_complete().is_err());
    assert!(OfferBook::from_documents_with_renewals(&r.state.snapshot, &docs, &decoded).is_err());
    let mut empty = receipt::RenewalReceiptAudit::default();
    empty.finish_replay()?;
    assert!(OfferBook::from_documents_with_renewals(&r.state.snapshot, &docs, &empty).is_err());
    assert!(
        OfferBook::from_documents_with_renewals(
            &r.state.snapshot,
            &[],
            &r.state.fold.renewal_receipts
        )
        .is_err()
    );
    let mut replay = crate::fold::FoldState::default();
    let mut entries = r.state.transactions.values().collect::<Vec<_>>();
    entries.sort_by_key(|tx| tx.journal_seq);
    for tx in entries {
        replay.fold_entry(tx.journal_seq, tx);
    }
    replay.renewal_receipts.finish_replay()?;
    assert_eq!(
        replay.renewal_receipts.get(req.operation)?,
        r.state.fold.renewal_receipts.get(req.operation)?
    );
    OfferBook::from_documents_with_renewals(&r.state.snapshot, &docs, &replay.renewal_receipts)?;
    Ok(())
}

#[test]
fn renewal_checkpoint_join_requires_both_actual_money_rows_even_after_hot_eviction() -> TestResult {
    let (mut r, req, docs) = funded(true)?;
    let p = prepared(&r, req.clone(), Transition::Grant, 5)?;
    install_money(&mut r, p)?;
    let rows = r
        .state
        .transactions
        .values()
        .filter(|tx| tx.idempotency.namespace.as_str() == NAMESPACE)
        .map(|tx| crate::checkpoint::IdemRow {
            ns: tx.idempotency.namespace.clone(),
            key: tx.idempotency.key.clone(),
            plan_hash: tx.plan_hash.iter().map(|v| format!("{v:02x}")).collect(),
            tx: tx.id,
            posted_at: tx.posted_at,
            seq: tx.journal_seq,
        })
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2);
    let raw = serde_json::to_vec(&r.state.fold.renewal_receipts)?;
    let decoded: receipt::RenewalReceiptAudit = serde_json::from_slice(&raw)?;
    assert!(decoded.require_complete().is_err());
    assert!(decoded.checkpoint_complete(&rows));
    assert!(!decoded.checkpoint_complete(&[]));
    for removed in 0..rows.len() {
        let mut missing = rows.clone();
        missing.remove(removed);
        assert!(!decoded.checkpoint_complete(&missing));
    }
    let mut duplicate = rows.clone();
    duplicate.push(rows.first().ok_or("row")?.clone());
    assert!(!decoded.checkpoint_complete(&duplicate));
    let mut crossed = rows.clone();
    crossed.first_mut().ok_or("row")?.tx = crate::TransactionId(u64::MAX);
    assert!(!decoded.checkpoint_complete(&crossed));
    let empty: receipt::RenewalReceiptAudit = serde_json::from_value(json!({
        "histories":{},"complete":true,"invalid":false,
    }))?;
    assert!(!empty.checkpoint_complete(&rows));
    // The actual journal-derived history survives removal of the hot map.
    // This tests retained evidence only; production replay/boot stays disabled.
    r.state.transactions.clear();
    let history = r.state.fold.renewal_receipts.get(req.operation)?;
    assert!(history.terminal().is_some());
    OfferBook::from_documents_with_renewals(
        &r.state.snapshot,
        &docs,
        &r.state.fold.renewal_receipts,
    )?;
    Ok(())
}

#[test]
fn renewal_replay_refuses_missing_fund_duplicate_and_changed_real_hold() -> TestResult {
    let (mut r, req, _) = funded(false)?;
    let p = prepared(&r, req.clone(), Transition::Refund, 5)?;
    let terminal = p.transaction.clone();
    install_money(&mut r, p)?;
    let fund = r
        .state
        .transactions
        .values()
        .find(|tx| tx.idempotency.key.as_str() == format!("{}/fund", req.operation.to_hex()))
        .ok_or("fund transaction")?;
    let mut missing = receipt::RenewalReceiptAudit::default();
    missing.observe(&terminal);
    assert!(missing.finish_replay().is_err());
    let mut duplicate = receipt::RenewalReceiptAudit::default();
    duplicate.observe(fund);
    duplicate.observe(fund);
    assert!(duplicate.finish_replay().is_err());
    let mut forged = terminal.clone();
    forged.hold_effects.first_mut().ok_or("hold")?.hold.amount = Amount::from_minor(99);
    let mut changed = receipt::RenewalReceiptAudit::default();
    changed.observe(fund);
    changed.observe(&forged);
    assert!(changed.finish_replay().is_err());
    // A later lawful row cannot erase a prior replay contradiction.
    changed.observe(&terminal);
    assert!(changed.finish_replay().is_err());
    Ok(())
}

#[test]
fn renewal_borrowed_audit_requires_combined_replay_and_exact_current_projection() -> TestResult {
    let mut r = Rig::new()?;
    cash(&mut r)?;
    let (offered, writes) = r.publish()?;
    let req = request(&offered);
    let docs = documents(&writes)?;
    assert!(r.state.renewal_audit().is_err());
    r.state.renewal_offers = OfferBook::from_documents(&r.state.snapshot, &docs)?;
    assert!(
        r.state.renewal_audit().is_err(),
        "unfunded-only audit is insufficient"
    );
    r.state.renewal_offers = OfferBook::from_documents_with_renewals(
        &r.state.snapshot,
        &docs,
        &r.state.fold.renewal_receipts,
    )?;
    let audit = r.state.renewal_audit()?;
    assert_eq!(audit.len(), 1);
    assert!(!audit.is_empty());
    assert!(audit.get(req.operation)?.history().is_none());
    assert_eq!(audit.offers().collect::<Result<Vec<_>, _>>()?.len(), 1);
    let p = prepared(&r, req.clone(), Transition::Fund, 4)?;
    install_money(&mut r, p)?;
    let audit = r.state.renewal_audit()?;
    let row = audit.get(req.operation)?;
    assert_eq!(row.operation(), offered.operation());
    assert_eq!(row.contract(), offered.contract());
    assert_eq!(row.offer(), &r.offer);
    assert_eq!(row.consent_hash(), offered.consent_hash());
    assert!(row.closure().is_none());
    assert_eq!(row.history().ok_or("history")?.funding().commit(), token(4));
    let mut changed = r
        .state
        .snapshot
        .contract(req.contract)
        .ok_or("contract")?
        .clone();
    changed.revision += 1;
    r.state.snapshot.put_contract(changed);
    assert!(r.state.renewal_audit()?.get(req.operation).is_err());
    r.state.frozen = true;
    assert!(r.state.renewal_audit().is_err());
    Ok(())
}
#[test]
fn renewal_new_namespace_legacy_refuses_before_exact_idempotency() -> TestResult {
    let (r, req, _) = funded(false)?;
    let history = r.state.fold.renewal_receipts.get(req.operation)?;
    // This is the exact already-posted Fund plan, so ordinary idempotency would
    // succeed if the closed-namespace guard ran too late.
    let plan = receipt::funding_plan(
        history.origin(),
        history.funding().commit(),
        history.funding().action(),
        history.funding().at_ic(),
    )?;
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
    assert!(matches!(
        crate::engine::validate(&plan, &r.state.snapshot, &env),
        PlanResult::Refused(crate::EconomyError::ManagedContract)
    ));
    Ok(())
}

#[test]
fn maximum_primitive_receipts_fit_their_real_strict_codecs() -> TestResult {
    // Structural sizing only: decoding these values grants no journal or
    // posting authority. Actual proof reconstruction is tested separately.
    let at = mush_core::ClockStamp {
        clock: mush_core::ClockRef::WorldIc {
            generation: u64::MAX,
        },
        at: i64::MIN,
    };
    let funding = json!({
        "format":1,"commit":CommitToken::from_bytes([255;16]),
        "action":action(FUND_ACTION,1)?,"contract":u64::MAX,"hold":u64::MAX,
        "transaction":u64::MAX,"journal_seq":u64::MAX,"consent_hash":vec![255u8;32],
        "at_ic":at,"posted_at_utc":i64::MIN,
    });
    let funding = receipt::RenewalFundingReceiptV1::decode(&serde_json::to_vec(&funding)?)?;
    let funding_bytes = serde_json::to_vec(&funding)?.len();
    assert!(funding_bytes <= receipt::MAX_RENEWAL_RECEIPT_BYTES);
    for grant in [false, true] {
        let terminal = json!({
            "format":1,"commit":CommitToken::from_bytes([254;16]),
            "action":action(if grant { GRANT_ACTION } else { REFUND_ACTION },1)?,
            "outcome":if grant { "granted" } else { "refunded" },
            "transaction":u64::MAX,"journal_seq":u64::MAX,"at_ic":at,
            "posted_at_utc":i64::MIN,"issued_at_ic":grant.then_some(&at),
        });
        let terminal = receipt::RenewalTerminalReceiptV1::decode(&serde_json::to_vec(&terminal)?)?;
        let terminal_bytes = serde_json::to_vec(&terminal)?.len();
        assert!(terminal_bytes <= receipt::MAX_RENEWAL_RECEIPT_BYTES);
        eprintln!(
            "F1_SIZE receipt_funding={funding_bytes} receipt_terminal={terminal_bytes} grant={grant}"
        );
    }
    Ok(())
}
#[test]
fn renewal_live_contract_account_and_book_fences_refuse_stale_bind() -> TestResult {
    for mode in ["account", "contract", "book"] {
        let mut r = Rig::new()?;
        cash(&mut r)?;
        let (offer, _) = r.publish()?;
        let prep = prepared(&r, request(&offer), Transition::Fund, 4)?;
        let (_, patch, _) =
            money_install::finish_images(&r.state, r.world.world_ic_now().clock, prep)?;
        match mode {
            "account" => {
                let mut a = r
                    .state
                    .snapshot
                    .account(r.offer.terms.payer)
                    .ok_or("payer")?
                    .clone();
                a.revision += 1;
                r.state.snapshot.put_account(a);
            }
            "contract" => {
                let mut c = r
                    .state
                    .snapshot
                    .contract(offer.contract())
                    .ok_or("contract")?
                    .clone();
                c.revision += 1;
                r.state.snapshot.put_contract(c);
            }
            _ => {
                let row = r
                    .state
                    .renewal_offers
                    .get(offer.operation())
                    .ok_or("offer")?
                    .clone();
                let generation = r.state.renewal_offers.next_generation()?;
                r.state.renewal_offers.install(row, generation);
            }
        }
        assert!(patch.bind(&mut r.state).is_err());
        assert_eq!(
            r.state
                .snapshot
                .account(r.offer.terms.payer)
                .ok_or("payer")?
                .held_amount,
            Amount::ZERO
        );
    }
    Ok(())
}

#[test]
fn renewal_receipt_strict_nested_decode_and_complete_terminal_margin() -> TestResult {
    let (mut r, req, _) = funded(true)?;
    let funding = r
        .state
        .fold
        .renewal_receipts
        .get(req.operation)?
        .funding()
        .clone();
    let encoded = serde_json::to_string(&funding)?;
    assert_eq!(
        receipt::RenewalFundingReceiptV1::decode(encoded.as_bytes())?,
        funding
    );
    assert!(
        receipt::RenewalFundingReceiptV1::decode(
            encoded.replacen("{", "{\"format\":1,", 1).as_bytes()
        )
        .is_err()
    );
    let mut bad = serde_json::to_value(&funding)?;
    bad.as_object_mut()
        .ok_or("receipt")?
        .insert("actor".into(), json!(GOD));
    assert!(receipt::RenewalFundingReceiptV1::decode(&serde_json::to_vec(&bad)?).is_err());
    bad = serde_json::to_value(&funding)?;
    bad.get_mut("at_ic")
        .and_then(|v| v.get_mut("clock"))
        .and_then(|v| v.get_mut("generation"))
        .ok_or("generation")?
        .clone_from(&json!(0));
    assert!(serde_json::from_value::<receipt::RenewalFundingReceiptV1>(bad).is_err());
    let prep = prepared(&r, req.clone(), Transition::Grant, 5)?;
    install_money(&mut r, prep)?;
    let history = r.state.fold.renewal_receipts.get(req.operation)?;
    let terminal = history.terminal().ok_or("terminal")?;
    let raw = serde_json::to_vec(terminal)?;
    assert_eq!(receipt::RenewalTerminalReceiptV1::decode(&raw)?, *terminal);
    let mut missing = serde_json::to_value(terminal)?;
    missing
        .as_object_mut()
        .ok_or("receipt")?
        .remove("issued_at_ic");
    assert!(serde_json::from_value::<receipt::RenewalTerminalReceiptV1>(missing).is_err());
    assert!(
        receipt::RenewalTerminalReceiptV1::decode(&vec![
            b' ';
            receipt::MAX_RENEWAL_RECEIPT_BYTES + 1
        ])
        .is_err()
    );
    assert!(raw.len() <= receipt::MAX_RENEWAL_RECEIPT_BYTES);
    Ok(())
}
fn replace_instrument(r: &mut Rig, length: usize) -> TestResult {
    let id = InstrumentId::new("x".repeat(length)).ok_or("instrument")?;
    let mut definition = r
        .state
        .snapshot
        .instrument(&r.offer.terms.instrument)
        .ok_or("definition")?
        .clone();
    definition.id = id.clone();
    r.state.snapshot.put_instrument(definition);
    let accounts = r.state.snapshot.accounts().cloned().collect::<Vec<_>>();
    for mut account in accounts {
        account.instrument = id.clone();
        r.state.snapshot.put_account(account);
    }
    r.offer.terms.instrument = id;
    Ok(())
}
#[test]
fn largest_actual_offer_keeps_both_complete_terminal_images_with_unchanged_f0_caps() -> TestResult {
    let mut r = Rig::new()?;
    r.six_roles()?;
    // Search the real F0 admission boundary, not a guessed payload length.
    let mut lo = 1usize;
    let mut hi = MAX_RENEWAL_OFFER_BYTES;
    while lo < hi {
        let mid = lo + (hi - lo + 1) / 2;
        replace_instrument(&mut r, mid)?;
        if r.prepare().is_ok() {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    replace_instrument(&mut r, lo)?;
    assert!(r.prepare().is_ok());
    replace_instrument(&mut r, lo + 1)?;
    assert!(r.prepare().is_err());
    replace_instrument(&mut r, lo)?;
    cash(&mut r)?;
    let (offer, offered) = r.publish()?;
    let req = request(&offer);
    let before_contract = serde_json::to_vec(
        r.state
            .snapshot
            .contract(offer.contract())
            .ok_or("contract")?,
    )?
    .len();
    let before_record =
        serde_json::to_vec(r.state.renewal_offers.get(req.operation).ok_or("record")?)?.len();
    assert!(before_record <= history::MAX_RECORD_BYTES - history::FUTURE_RESERVE_BYTES);
    assert!(before_contract <= history::MAX_CONTRACT_BYTES - history::FUTURE_RESERVE_BYTES);
    let prep = prepared(&r, req.clone(), Transition::Fund, 4)?;
    let (_, funded_images) = install_money(&mut r, prep)?;
    let h = r.state.fold.renewal_receipts.get(req.operation)?;
    h.require_terminal_room()?;
    let history_bytes = serde_json::to_vec(h)?.len();
    assert!(history_bytes <= receipt::MAX_HISTORY_BYTES - receipt::TERMINAL_RESERVE_BYTES);
    for transition in [Transition::Grant, Transition::Refund] {
        let prep = prepared(&r, req.clone(), transition, 5)?;
        assert!(serde_json::to_vec(&prep.transaction)?.len() <= crate::engine::MAX_ENTRY_BYTES);
        let (_, patch, writes) =
            money_install::finish_images(&r.state, r.world.world_ic_now().clock, prep)?;
        let complete = serde_json::to_vec(&writes)?.len();
        assert!(complete <= mush_core::authoritative::MAX_AUTHORITATIVE_BUNDLE_BYTES);
        eprintln!(
            "F1_SIZE terminal={} complete_writes={} offered_record={} offered_contract={} funded_history={}",
            if transition == Transition::Grant {
                "grant"
            } else {
                "refund"
            },
            complete,
            before_record,
            before_contract,
            history_bytes
        );
        drop(patch);
    }
    assert!(!offered.is_empty() && !funded_images.is_empty());
    Ok(())
}
#[test]
fn renewal_zero_cash_credit_changed_owner_and_approval_rule_never_create_escrow() -> TestResult {
    for mode in ["credit", "owner", "approval"] {
        let mut r = Rig::new()?;
        if mode == "credit" {
            r.state.fold.renewal_receipts.finish_replay()?;
        } else {
            cash(&mut r)?;
        }
        let (offer, _) = r.publish()?;
        let mut payer = r
            .state
            .snapshot
            .account(r.offer.terms.payer)
            .ok_or("payer")?
            .clone();
        match mode {
            "credit" => {
                payer.credit = CreditPolicy {
                    allow_negative: true,
                    floor: Some(Amount::from_minor(-1000)),
                }
            }
            "owner" => payer.owner = PrincipalId::Object(GOD),
            _ => {
                payer.approval = Some(crate::record::ApprovalRule {
                    threshold: Amount::ZERO,
                    policy: mush_org::ApprovalPolicy::Threshold {
                        approvals: 2,
                        from_role: None,
                    },
                    window: None,
                })
            }
        }
        r.state.snapshot.put_account(payer);
        assert!(prepared(&r, request(&offer), Transition::Fund, 4).is_err());
        assert_eq!(r.state.snapshot.holds().count(), 0);
    }
    Ok(())
}

#[path = "funded/reoffer.rs"]
mod reoffer;

#[path = "funded/service.rs"]
mod service;

/// Native owner history is permanent even when the ordinary hold-GC window
/// elapses. The actual owner images are placed in the actual registered Economy
/// slot solely to run its real tick; this does not claim a paired licence boot.
#[test]
fn renewal_terminal_holds_survive_actual_sweep_and_complete_cold_reconstruction() -> TestResult {
    for transition in [Transition::Grant, Transition::Refund] {
        let (mut r, req, docs) = funded(false)?;
        let prep = prepared(&r, req.clone(), transition, 5)?;
        install_money(&mut r, prep)?;
        let actual = r.state.fold.renewal_receipts.get(req.operation)?.clone();
        let hold = actual.funding().hold();
        let terminal_at = r
            .state
            .hold_terminal_at(hold)
            .ok_or("actual terminal timestamp")?;
        assert!(
            !r.state
                .snapshot
                .hold(hold)
                .ok_or("actual terminal hold")?
                .status
                .is_held()
        );
        let mut entries = r.state.transactions.values().cloned().collect::<Vec<_>>();
        entries.sort_by_key(|tx| tx.journal_seq);
        let now = terminal_at
            .checked_add(crate::HOLD_GC_AFTER_SECS + 60)
            .ok_or("gc time")?;
        let mut engine = boxed_test_engine(
            r.world,
            mush_core::Registry::new(),
            vec![
                Box::new(mush_org::OrgModule::new()),
                Box::new(crate::EconomyModule::new()),
            ],
        )?;
        engine.modules_loaded(
            Default::default(),
            &mut mush_core::Outbox::new(),
            &mut mush_core::Effects::new(),
        )?;
        *engine
            .world
            .module_state_mut::<EconomyStore>()
            .ok_or("actual Economy slot")? = r.state;
        let mut world = mush_testkit::TestWorld::from_engine(*engine);
        let _ = world.take_module_ops();
        world.set_now(now);
        let _ = world.fire_due(now.checked_mul(1000).ok_or("gc milliseconds")?);
        let state = world
            .world()
            .module_state::<EconomyStore>()
            .ok_or("actual store")?;
        assert_eq!(state.last_sweep().0, now, "actual registered sweep ran");
        assert!(!state.frozen);
        assert!(
            state.snapshot.hold(hold).is_some(),
            "renewal audit hold must remain"
        );
        assert!(state.fold.journal_holds.contains_key(&hold));
        actual.verify_documents(
            state.snapshot.contract(req.contract).ok_or("contract")?,
            state.snapshot.hold(hold).ok_or("hold")?,
        )?;
        let mut replay = crate::fold::FoldState::default();
        for tx in &entries {
            replay.fold_entry(tx.journal_seq, tx);
        }
        let mut snapshot = state.snapshot.clone();
        let mut contracts = state
            .snapshot
            .contracts()
            .map(|c| (c.id, c.clone()))
            .collect();
        let holds = state.snapshot.holds().map(|h| (h.id, h.clone())).collect();
        let book = crate::managed_renewal::bridge::recover(
            &mut replay,
            &mut snapshot,
            &mut contracts,
            &holds,
            &docs,
            true,
        )?;
        assert!(book.combined_verified());
        assert!(replay.renewal_receipts.get(req.operation)? == &actual);
        let retained = state.snapshot.hold(hold).ok_or("hold")?.clone();
        let mut missing = holds.clone();
        assert_eq!(missing.remove(&hold), Some(retained));
        assert!(
            crate::managed_renewal::bridge::recover(
                &mut replay,
                &mut snapshot,
                &mut contracts,
                &missing,
                &docs,
                true
            )
            .is_err(),
            "cold audit requires exact retained hold"
        );
    }
    Ok(())
}

#[inline(never)]
fn boxed_test_engine(
    world: mush_core::World,
    registry: mush_core::Registry,
    modules: Vec<Box<dyn mush_core::GameModule>>,
) -> Result<Box<mush_core::Engine>, mush_core::ModError> {
    mush_core::Engine::with_modules(world, registry, modules).map(Box::new)
}

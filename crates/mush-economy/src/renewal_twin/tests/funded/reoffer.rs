//! Actual owner create/close and money plans. No paired commercial head or
//! commercial predecessor is fabricated by these isolated E4 fixtures.
use super::*;
use crate::renewal_twin::RenewalAttemptV1;

fn retry<I: Rig>(
    r: &Fixture<I>,
    previous: u64,
    commit: u64,
) -> Result<Prepared<I>, ParticipantError> {
    install::retry::<I>(
        &r.state,
        &r.context(false, token(commit)),
        r.offer.clone(),
        token(previous),
    )
}
fn publish_retry<I: Rig>(
    r: &mut Fixture<I>,
    previous: u64,
    commit: u64,
) -> TestResult<RenewalOfferOutput<I>> {
    let (out, patch, _) = retry(r, previous, commit)?;
    patch.recheck_in(&r.state, &r.context(false, token(commit)))?;
    patch.bind(&mut r.state)?.apply();
    Ok(out)
}
fn close_at<I: Rig>(r: &mut Fixture<I>, operation: u64, commit: u64) -> TestResult {
    let (_, patch, _) = install::close::<I>(
        &r.state,
        &r.context(true, token(commit)),
        token(operation),
        UnfundedCloseReason::Declined,
    )?;
    patch.recheck_in(&r.state, &r.context(true, token(commit)))?;
    patch.bind(&mut r.state)?.apply();
    Ok(())
}
fn all_documents<I: Rig>(r: &Fixture<I>) -> TestResult<Vec<(String, Vec<u8>)>> {
    I::book(&r.state)
        .operations()
        .map(|op| {
            Ok((
                history::key::<I>(op),
                serde_json::to_vec(I::book(&r.state).get(op).ok_or("offer")?)?,
            ))
        })
        .collect()
}
fn cold<I: Rig>(
    r: &Fixture<I>,
    docs: &[(String, Vec<u8>)],
) -> Result<OfferBook<I>, ParticipantError> {
    OfferBook::<I>::from_documents_with_renewals(
        &r.state.snapshot,
        docs,
        I::receipts(&r.state.fold),
    )
}

pub(crate) fn actual_unfunded_closures_allow_only_explicit_fresh_latest_buyer_retry<I: Rig>()
-> TestResult {
    for reason in [UnfundedCloseReason::Declined, UnfundedCloseReason::Expired] {
        let mut r = Fixture::<I>::new()?;
        cash(&mut r)?;
        let (first, _) = r.publish()?;
        check_eq!(first.attempt(), RenewalAttemptV1::Automatic);
        check!(retry(&r, 3, 6).is_err(), "unaccepted is not terminal");
        if reason == UnfundedCloseReason::Expired {
            r.world.set_now(r.offer.terms().expires_at().at);
        }
        let (_, patch, _) = r.close(reason)?;
        patch.bind(&mut r.state)?.apply();
        let before = serde_json::to_vec(I::book(&r.state).get(token(3)).ok_or("old offer")?)?;
        // A fresh disclosed interval is necessary after actual expiration.
        *I::terms(&mut r.offer).not_before = r.world.world_ic_now();
        I::terms(&mut r.offer).expires_at.at = r
            .offer
            .terms()
            .not_before()
            .at
            .checked_add(100)
            .ok_or("time")?;
        let next_contract = r.state.next_contract;
        check!(
            install::offer::<I>(&r.state, &r.context(false, token(6)), r.offer.clone()).is_err()
        );
        let second = publish_retry(&mut r, 3, 6)?;
        check_eq!(
            second.attempt(),
            RenewalAttemptV1::BuyerRetry {
                previous_operation: token(3)
            }
        );
        check_eq!(second.contract(), ContractId(next_contract));
        check_ne!(second.contract(), first.contract());
        check_ne!(second.consent_hash(), first.consent_hash());
        check_eq!(
            serde_json::to_vec(I::book(&r.state).get(token(3)).ok_or("old offer")?)?,
            before
        );
        check!(
            retry(&r, 3, 7).is_err(),
            "older terminal cannot fork the chain"
        );
        check!(retry(&r, 6, 7).is_err(), "new pending offer is retained");
        close_at(&mut r, 6, 7)?;
        publish_retry(&mut r, 6, 8)?;
        let mut docs = all_documents(&r)?;
        docs.reverse();
        *I::book_mut(&mut r.state) = cold(&r, &docs)?;
        let audit = audit::read::<I>(&r.state)?;
        check_eq!(audit.len(), 3);
        let row = audit.get(token(8))?;
        check_eq!(row.format(), 2);
        check_eq!(
            row.attempt(),
            RenewalAttemptV1::BuyerRetry {
                previous_operation: token(6)
            }
        );
        check!(row.history().is_none());
        check!(
            retry(&r, 3, 9).is_err(),
            "cold order cannot roll back latest attempt"
        );
    }
    Ok(())
}

pub(crate) fn actual_refund_then_retry_grant_retains_both_journal_histories_and_old_closed_proof<
    I: Rig,
>() -> TestResult {
    let (mut r, first, _) = funded::<I>(true)?;
    check!(
        retry(&r, 3, 6).is_err(),
        "funded without terminal is not retryable"
    );
    let prep = prepared(&r, first.clone(), Transition::Refund, 5)?;
    install_money(&mut r, prep)?;
    let old = I::receipts(&r.state.fold).get(token(3))?.clone();
    let offered = publish_retry(&mut r, 3, 6)?;
    let second = request(&offered);
    let prep = prepared(&r, second.clone(), Transition::Fund, 7)?;
    install_money(&mut r, prep)?;
    let prep = prepared(&r, second, Transition::Grant, 8)?;
    install_money(&mut r, prep)?;
    check_eq!(I::receipts(&r.state.fold).get(token(3))?, &old);
    let current = I::receipts(&r.state.fold).get(token(6))?;
    check_ne!(old.funding().hold(), current.funding().hold());
    check_eq!(
        old.terminal().ok_or("old refund")?.outcome(),
        RenewalOutcome::Refunded
    );
    check_eq!(
        current.terminal().ok_or("new grant")?.outcome(),
        RenewalOutcome::Granted
    );
    check!(
        retry(&r, 6, 9).is_err(),
        "Grant never authorizes same-generation retry"
    );
    let mut docs = all_documents(&r)?;
    docs.reverse();
    *I::book_mut(&mut r.state) = cold(&r, &docs)?;
    check_eq!(
        audit::read::<I>(&r.state)?
            .offers()
            .collect::<Result<Vec<_>, _>>()?
            .len(),
        2
    );
    check!(OfferBook::<I>::from_documents(&r.state.snapshot, &docs).is_err());
    let mut replay = crate::fold::FoldState::default();
    let mut txs = r.state.transactions.values().collect::<Vec<_>>();
    txs.sort_by_key(|tx| tx.journal_seq);
    for tx in txs {
        replay.fold_entry(tx.journal_seq, tx);
    }
    I::receipts_mut(&mut replay).finish_replay()?;
    check_eq!(replay, r.state.fold);
    OfferBook::<I>::from_documents_with_renewals(&r.state.snapshot, &docs, I::receipts(&replay))?;
    Ok(())
}

pub(crate) fn retry_refuses_crossed_principal_basis_clock_and_stale_terminal_before_custody<
    I: Rig,
>() -> TestResult {
    let (mut r, first, _) = funded::<I>(false)?;
    let prep = prepared(&r, first, Transition::Refund, 5)?;
    install_money(&mut r, prep)?;
    let first_contract = I::book(&r.state).get(token(3)).ok_or("prior row")?.contract;
    let stranger = I::stranger(&mut r)?;
    let state = r.state.clone();
    let offer = r.offer.clone();
    for case in 0..8 {
        r.state = state.clone();
        r.offer = offer.clone();
        match case {
            0 => *I::basis(&mut r.offer).product_digest = DefinitionDigest::from_bytes([33; 32]),
            1 => *I::basis(&mut r.offer).predecessor_generation = 2,
            2 => *I::terms(&mut r.offer).buyer = stranger.into(),
            3 => *I::receipts_mut(&mut r.state.fold) = receipt::RenewalReceiptAudit::<I>::default(),
            4 => {
                let mut c = r
                    .state
                    .snapshot
                    .contract(first_contract)
                    .ok_or("contract")?
                    .clone();
                c.revision = c.revision.checked_add(1).ok_or("revision")?;
                r.state.snapshot.put_contract(c);
            }
            5 => *I::terms(&mut r.offer).title_revision = 2,
            6 => *I::terms(&mut r.offer).expires_at = r.world.world_ic_now(),
            7 => {
                I::terms(&mut r.offer).not_before.clock = mush_core::ClockRef::WorldIc {
                    generation: u64::MAX,
                }
            }
            _ => return Err("case".into()),
        }
        check!(retry(&r, 3, 6).is_err(), "crossed retry case {case}");
    }
    r.state = state.clone();
    r.offer = offer;
    let mut context = r.context(false, token(6));
    context.actor = GOD;
    check!(install::retry::<I>(&r.state, &context, r.offer.clone(), token(3)).is_err());
    let (_, patch, _) = retry(&r, 3, 6)?;
    *I::receipts_mut(&mut r.state.fold) = receipt::RenewalReceiptAudit::<I>::default();
    check!(
        patch
            .recheck_in(&r.state, &r.context(false, token(6)))
            .is_err()
    );
    check!(
        patch.bind(&mut r.state).is_err(),
        "actual terminal proof must survive until bind"
    );
    r.state = state;
    let (_, pending, _) = retry(&r, 3, 6)?;
    publish_retry(&mut r, 3, 7)?;
    check!(
        pending.bind(&mut r.state).is_err(),
        "a different winning retry consumes latest link"
    );
    Ok(())
}

pub(crate) fn legacy_wire_is_exact_and_retry_shape_forbids_missing_null_or_automatic_attempts<
    I: Rig,
>() -> TestResult {
    let mut r = Fixture::<I>::new()?;
    cash(&mut r)?;
    r.publish()?;
    let old = I::book(&r.state).get(token(3)).ok_or("old")?;
    // Original serializer field order and omission are retained exactly.
    #[derive(serde::Serialize)]
    struct Original<'a, O> {
        format: u16,
        operation: CommitToken,
        action: &'a DefinitionRef,
        offer: &'a O,
        contract: ContractId,
        consent_hash: [u8; 32],
        created_at_ic: &'a ClockStamp,
        created_at_utc: i64,
        closure: Option<&'a history::Closure>,
    }
    let original = Original {
        format: old.format,
        operation: old.operation,
        action: &old.action,
        offer: &old.offer,
        contract: old.contract,
        consent_hash: old.consent_hash,
        created_at_ic: &old.created_at_ic,
        created_at_utc: old.created_at_utc,
        closure: old.closure.as_ref(),
    };
    check_eq!(serde_json::to_vec(old)?, serde_json::to_vec(&original)?);
    let old_value = serde_json::to_value(old)?;
    close_at(&mut r, 3, 4)?;
    publish_retry(&mut r, 3, 6)?;
    let new = I::book(&r.state).get(token(6)).ok_or("new")?;
    let new_value = serde_json::to_value(new)?;
    check_eq!(new.format, 2);
    for mut value in [old_value.clone(), new_value.clone()] {
        value
            .as_object_mut()
            .ok_or("object")?
            .insert("attempt".into(), serde_json::Value::Null);
        check!(serde_json::from_value::<OfferRecord<I>>(value).is_err());
    }
    for (mut value, attempt) in [
        (
            old_value,
            Some(json!({"kind":"buyer_retry","previous_operation":token(4)})),
        ),
        (new_value.clone(), None),
        (new_value.clone(), Some(json!({"kind":"automatic"}))),
        (
            new_value,
            Some(json!({"kind":"buyer_retry","previous_operation":token(6)})),
        ),
    ] {
        let obj = value.as_object_mut().ok_or("object")?;
        if let Some(attempt) = attempt {
            obj.insert("attempt".into(), attempt);
        } else {
            obj.remove("attempt");
        }
        let row: OfferRecord<I> = serde_json::from_value(value)?;
        check!(row.validate().is_err());
    }
    let bytes = serde_json::to_string(new)?;
    let duplicate = bytes.replacen(
        "\"attempt\":",
        "\"attempt\":{\"kind\":\"automatic\"},\"attempt\":",
        1,
    );
    check!(serde_json::from_str::<OfferRecord<I>>(&duplicate).is_err());
    Ok(())
}

pub(crate) fn complete_recovery_rejects_missing_crossed_branched_and_cyclic_retry_links<I: Rig>()
-> TestResult {
    let mut r = Fixture::<I>::new()?;
    cash(&mut r)?;
    r.publish()?;
    close_at(&mut r, 3, 4)?;
    publish_retry(&mut r, 3, 6)?;
    close_at(&mut r, 6, 7)?;
    publish_retry(&mut r, 6, 8)?;
    let rows = I::book(&r.state)
        .operations()
        .map(|op| I::book(&r.state).get(op).cloned().ok_or("row"))
        .collect::<Result<Vec<_>, _>>()?;
    let baseline = all_documents(&r)?;
    cold(&r, &baseline)?;
    for case in 0..7 {
        let mut altered = rows.clone();
        let second = altered
            .iter_mut()
            .find(|row| row.operation == token(6))
            .ok_or("second")?;
        match case {
            0 => {
                second.attempt = Some(RenewalAttemptV1::BuyerRetry {
                    previous_operation: token(99),
                })
            }
            1 => {
                second.attempt = Some(RenewalAttemptV1::BuyerRetry {
                    previous_operation: token(8),
                });
                let last = altered
                    .iter_mut()
                    .find(|row| row.operation == token(8))
                    .ok_or("last")?;
                last.closure = Some(history::Closure {
                    commit: token(9),
                    action: r.close.clone(),
                    at_ic: r.world.world_ic_now(),
                    at_utc: r.world.now(),
                    reason: UnfundedCloseReason::Declined,
                });
            }
            2 => {
                *I::basis(&mut second.offer).product_digest = DefinitionDigest::from_bytes([91; 32])
            }
            3 => *I::terms(&mut second.offer).buyer = ObjId(u32::MAX).into(),
            4 => second.created_at_utc = 999,
            5 => second.closure = None,
            6 => {
                let last = altered
                    .iter_mut()
                    .find(|row| row.operation == token(8))
                    .ok_or("last")?;
                last.attempt = Some(RenewalAttemptV1::BuyerRetry {
                    previous_operation: token(3),
                });
            }
            _ => return Err("case".into()),
        }
        let mut snapshot = r.state.snapshot.clone();
        let mut docs = Vec::new();
        for row in &mut altered {
            row.consent_hash = contract(row)?.consent_hash;
            row.validate()?;
            snapshot.put_contract(contract(row)?);
            row.verify_documents(&snapshot)?;
            docs.push((history::key::<I>(row.operation), serde_json::to_vec(row)?));
        }
        docs.reverse();
        check!(
            OfferBook::<I>::from_documents_with_renewals(
                &snapshot,
                &docs,
                I::receipts(&r.state.fold)
            )
            .is_err(),
            "linked corruption case {case}"
        );
    }
    let mut missing = baseline.clone();
    missing.retain(|(key, _)| key != &history::key::<I>(token(3)));
    check!(cold(&r, &missing).is_err());
    let mut without_root = r.state.snapshot.clone();
    without_root.remove_contract(I::book(&r.state).get(token(3)).ok_or("root")?.contract);
    check!(
        OfferBook::<I>::from_documents_with_renewals(
            &without_root,
            &missing,
            I::receipts(&r.state.fold)
        )
        .is_err(),
        "orphan must refuse even without an orphan contract"
    );
    let mut duplicate = baseline.clone();
    duplicate.push(baseline.first().ok_or("first")?.clone());
    check!(cold(&r, &duplicate).is_err());
    Ok(())
}

pub(crate) fn newer_automatic_predecessor_prevents_returning_to_old_retry_head<I: Rig>()
-> TestResult {
    let mut r = Fixture::<I>::new()?;
    cash(&mut r)?;
    r.publish()?;
    close_at(&mut r, 3, 4)?;
    let old = r.offer.clone();
    *I::basis(&mut r.offer).predecessor_generation = 2;
    *I::basis(&mut r.offer).next_generation = 3;
    // This is only an E4 economic fixture. F3 separately must prove the actual
    // commercial current generation and prior successful grant.
    let (_, patch, _) =
        install::offer::<I>(&r.state, &r.context(false, token(6)), r.offer.clone())?;
    patch.bind(&mut r.state)?.apply();
    r.offer = old;
    check!(retry(&r, 3, 7).is_err());
    check!(install::offer::<I>(&r.state, &r.context(false, token(7)), r.offer.clone()).is_err());
    let mut docs = all_documents(&r)?;
    docs.reverse();
    *I::book_mut(&mut r.state) = cold(&r, &docs)?;
    check!(retry(&r, 3, 7).is_err());
    Ok(())
}

pub(crate) fn complete_retry_closure_fits_a_full_legacy_offer_without_raising_any_cap<I: Rig>()
-> TestResult {
    let mut r = Fixture::<I>::new()?;
    I::widen(&mut r)?;
    r.publish()?;
    let original = I::book(&r.state).get(token(3)).ok_or("origin")?.clone();
    // Pure maximum-shape sizing, not an authority/current-account fixture.
    // Find the actual old validator boundary using a permitted JSON
    // string field; the fixed identity and generation widths are maximized.
    let mut original = original;
    original.contract = ContractId(u64::MAX);
    *I::basis(&mut original.offer).licence = u64::MAX;
    *I::basis(&mut original.offer).predecessor_generation = u64::MAX - 1;
    *I::basis(&mut original.offer).next_generation = u64::MAX;
    *I::terms(&mut original.offer).price = Amount::from_minor(i64::MAX);
    *I::terms(&mut original.offer).buyer = ObjId(u32::MAX).into();
    *I::terms(&mut original.offer).charter = ObjId(u32::MAX - 2);
    *I::terms(&mut original.offer).title_revision = u64::MAX;
    *I::terms(&mut original.offer).payer = AccountId(u64::MAX);
    I::terms(&mut original.offer).remainder.account = AccountId(u64::MAX - 1);
    I::terms(&mut original.offer)
        .registry
        .recipient
        .as_mut()
        .ok_or("registry")?
        .account = AccountId(u64::MAX - 2);
    I::terms(&mut original.offer)
        .venue
        .as_mut()
        .and_then(|v| v.recipient.as_mut())
        .ok_or("venue")?
        .account = AccountId(u64::MAX - 3);
    I::widest(&mut original.offer)?;
    *I::terms(&mut original.offer).not_before = ClockStamp {
        clock: mush_core::ClockRef::WorldIc {
            generation: u64::MAX,
        },
        at: i64::MAX - MAX_RENEWAL_OFFER_IC_SECONDS,
    };
    *I::terms(&mut original.offer).expires_at = ClockStamp {
        clock: original.offer.terms().not_before().clock.clone(),
        at: i64::MAX,
    };
    original.created_at_ic = original.offer.terms().not_before().clone();
    original.created_at_utc = i64::MAX;
    let mut low = 0usize;
    let mut high = history::MAX_RECORD_BYTES;
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        let mut row = original.clone();
        *I::terms(&mut row.offer).instrument =
            InstrumentId::new(format!("cr{}", "a".repeat(mid))).ok_or("instrument")?;
        row.consent_hash = contract(&row)?.consent_hash;
        if row.validate().is_ok() {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    *I::terms(&mut original.offer).instrument =
        InstrumentId::new(format!("cr{}", "a".repeat(low))).ok_or("instrument")?;
    original.consent_hash = contract(&original)?.consent_hash;
    original.validate()?;
    let mut oversized = original.clone();
    *I::terms(&mut oversized.offer).instrument =
        InstrumentId::new(format!("{}a", original.offer.terms().instrument()))
            .ok_or("instrument")?;
    oversized.consent_hash = contract(&oversized)?.consent_hash;
    check!(
        oversized.validate().is_err(),
        "actual legacy admission boundary"
    );
    let legacy_bytes = serde_json::to_vec(&original)?.len();
    let mut retry = original.clone();
    retry.format = 2;
    retry.operation = token(6);
    retry.attempt = Some(RenewalAttemptV1::BuyerRetry {
        previous_operation: token(3),
    });
    retry.consent_hash = contract(&retry)?.consent_hash;
    retry.validate()?;
    let open_bytes = serde_json::to_vec(&retry)?.len();
    check!(open_bytes > legacy_bytes);
    check!(open_bytes <= history::MAX_RECORD_BYTES);
    for reason in [UnfundedCloseReason::Declined, UnfundedCloseReason::Expired] {
        let mut terminal = retry.clone();
        terminal.closure = Some(history::Closure {
            commit: token(7),
            action: r.close.clone(),
            at_ic: ClockStamp {
                clock: terminal.created_at_ic.clock.clone(),
                at: i64::MAX,
            },
            at_utc: i64::MAX,
            reason,
        });
        terminal.validate()?;
        let actual = contract(&terminal)?;
        let writes = vec![
            crate::registry_filing::json_write(
                history::key::<I>(terminal.operation).into(),
                &terminal,
            )?,
            crate::registry_filing::json_write(
                crate::api::contract_key(terminal.contract),
                &actual,
            )?,
        ];
        check!(serde_json::to_vec(&terminal)?.len() <= history::MAX_RECORD_BYTES);
        check!(serde_json::to_vec(&actual)?.len() <= history::MAX_CONTRACT_BYTES);
        let envelope = serde_json::to_vec(&writes)?;
        check!(envelope.len() <= 32_768);
        check_eq!(
            serde_json::from_slice::<Vec<ReplayWrite>>(&envelope)?,
            writes
        );
        check_eq!(actual.consent_hash, contract(&retry)?.consent_hash);
    }
    check_eq!(history::MAX_RECORD_BYTES, 8192);
    check_eq!(history::MAX_CONTRACT_BYTES, 8192);
    check_eq!(history::FUTURE_RESERVE_BYTES, 2048);
    Ok(())
}

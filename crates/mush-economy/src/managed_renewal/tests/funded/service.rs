//! Real isolated owner images and complete book reconstruction. These tests do
//! not substitute data for the Market service intent or paired Roll evidence.
use super::*;
use crate::managed_renewal::{
    RenewalServiceCloseReason as Reason,
    service::{self as service_proof, ServiceRecord},
};

fn rows(r: &Rig) -> TestResult<Vec<(String, Vec<u8>)>> {
    let mut rows = Vec::new();
    for operation in r.state.renewal_offers.operations() {
        rows.push((
            history::key(operation),
            serde_json::to_vec(r.state.renewal_offers.get(operation).ok_or("origin")?)?,
        ));
        if let Some(record) = r.state.renewal_offers.service(operation) {
            rows.push((service_proof::key(operation), serde_json::to_vec(record)?));
        }
    }
    Ok(rows)
}
fn cold(r: &mut Rig) -> TestResult {
    let mut documents = rows(r)?;
    documents.reverse();
    r.state.renewal_offers = OfferBook::from_documents_with_renewals(
        &r.state.snapshot,
        &documents,
        &r.state.fold.renewal_receipts,
    )?;
    Ok(())
}
fn ready() -> TestResult<Rig> {
    let mut r = Rig::new()?;
    r.publish()?;
    r.state.fold.renewal_receipts.finish_replay()?;
    cold(&mut r)?;
    Ok(r)
}
fn changed_clock(r: &mut Rig) -> TestResult {
    let next = r.world.next_world_timeline_epoch(2)?;
    r.world.append_world_timeline_epoch(next)?;
    Ok(())
}
fn service_context<'a>(
    r: &'a Rig,
    action: &'a DefinitionRef,
    principal: &'a mush_core::PrincipalId,
    commit: u64,
) -> OfferContext<'a> {
    OfferContext {
        world: &r.world,
        actor: GOD,
        principal,
        action,
        commit: token(commit),
    }
}
fn prepare(r: &Rig, reason: Reason) -> TestResult<Prepared> {
    let action = action(SERVICE_CLOSE_ACTION, 1)?;
    let principal = mush_core::PrincipalId::Object { id: GOD };
    Ok(install::service_close(
        &r.state,
        &service_context(r, &action, &principal, 4),
        token(3),
        reason,
    )?)
}
fn publish(r: &mut Rig, reason: Reason) -> TestResult<Vec<ReplayWrite>> {
    let (output, patch, writes) = prepare(r, reason)?;
    assert_eq!(output.closed(), None);
    assert_eq!(
        output.service_closed().ok_or("service output")?.reason(),
        reason
    );
    assert_eq!(output.commit(), token(4));
    let action = action(SERVICE_CLOSE_ACTION, 1)?;
    let principal = mush_core::PrincipalId::Object { id: GOD };
    patch.recheck_in(&r.state, &service_context(r, &action, &principal, 4))?;
    patch.bind(&mut r.state)?.apply();
    Ok(writes)
}

#[test]
fn service_both_causes_preserve_origin_bytes_and_all_money_through_cold_replay() -> TestResult {
    for reason in [Reason::DeadlineReached, Reason::ClockInvalidated] {
        let mut r = ready()?;
        let origin = serde_json::to_vec(r.state.renewal_offers.get(token(3)).ok_or("origin")?)?;
        let before = r.state.clone();
        match reason {
            Reason::DeadlineReached => r.world.set_now(r.offer.terms.expires_at.at),
            Reason::ClockInvalidated => changed_clock(&mut r)?,
        }
        let writes = publish(&mut r, reason)?;
        same_money(&before, &r.state);
        assert_eq!(r.state.next_contract, before.next_contract);
        assert_eq!(
            origin,
            serde_json::to_vec(r.state.renewal_offers.get(token(3)).ok_or("origin")?)?
        );
        assert_eq!(writes.len(), 2);
        assert!(!writes.iter().any(|w|matches!(w,ReplayWrite::ModuleJsonSet{key,..} if key.as_str().starts_with(history::PREFIX))));
        let contract = r
            .state
            .snapshot
            .contract(ContractId(before.next_contract - 1))
            .ok_or("contract")?;
        assert!(contract.acceptances.is_empty() && contract.escrow.is_empty());
        assert_eq!(contract.revision, 2);
        assert_eq!(
            contract.state,
            if reason == Reason::DeadlineReached {
                ContractState::Expired
            } else {
                ContractState::Cancelled
            }
        );
        cold(&mut r)?;
        let view = r.state.renewal_audit()?.get(token(3))?;
        assert!(view.history().is_none() && view.closure().is_none());
        assert_eq!(
            view.service_closed().ok_or("service history")?.reason(),
            reason
        );
        assert!(prepare(&r, reason).is_err());
    }
    Ok(())
}

#[test]
fn service_cleans_unavailable_buyer_but_never_borrows_buyer_or_funded_authority() -> TestResult {
    let mut r = ready()?;
    r.world
        .get_mut(r.offer.terms.buyer)
        .ok_or("buyer")?
        .flags
        .insert(mush_core::Flags::GOING);
    r.world.set_now(r.offer.terms.expires_at.at);
    let action = action(SERVICE_CLOSE_ACTION, 1)?;
    let principal = mush_core::PrincipalId::Object { id: GOD };
    let mut context = service_context(&r, &action, &principal, 4);
    context.actor = r.offer.terms.buyer;
    assert!(install::service_close(&r.state, &context, token(3), Reason::DeadlineReached).is_err());
    context.actor = GOD;
    context.principal = &r.principal;
    assert!(install::service_close(&r.state, &context, token(3), Reason::DeadlineReached).is_err());
    publish(&mut r, Reason::DeadlineReached)?;
    let (mut funded, _, _) = funded(true)?;
    cold(&mut funded)?;
    funded.world.set_now(funded.offer.terms.expires_at.at);
    assert!(prepare(&funded, Reason::DeadlineReached).is_err());
    Ok(())
}

#[test]
fn service_clocks_and_prepared_owner_fences_refuse_wrong_or_changed_evidence() -> TestResult {
    let mut r = ready()?;
    assert!(prepare(&r, Reason::DeadlineReached).is_err());
    assert!(prepare(&r, Reason::ClockInvalidated).is_err());
    changed_clock(&mut r)?;
    assert!(
        r.close(UnfundedCloseReason::Expired).is_err(),
        "old Expired remains same-generation only"
    );
    assert!(prepare(&r, Reason::DeadlineReached).is_err());
    let (_, patch, _) = prepare(&r, Reason::ClockInvalidated)?;
    let action = action(SERVICE_CLOSE_ACTION, 1)?;
    let principal = mush_core::PrincipalId::Object { id: GOD };
    r.world.set_now(999);
    assert!(
        patch
            .recheck_in(&r.state, &service_context(&r, &action, &principal, 4))
            .is_err()
    );
    r.world.set_now(1000);
    changed_clock(&mut r)?;
    assert!(
        patch
            .recheck_in(&r.state, &service_context(&r, &action, &principal, 4))
            .is_err()
    );
    let (_, patch, _) = prepare(&r, Reason::ClockInvalidated)?;
    let before = r.state.clone();
    let contract = r
        .state
        .snapshot
        .contract(ContractId(r.state.next_contract - 1))
        .ok_or("contract")?
        .clone();
    let mut changed = contract.clone();
    changed.revision += 1;
    r.state.snapshot.put_contract(changed);
    assert!(patch.bind(&mut r.state).is_err());
    same_money(&before, &r.state);
    r.state.snapshot.put_contract(contract);
    publish(&mut r, Reason::ClockInvalidated)?;
    Ok(())
}

#[test]
fn service_history_rejects_missing_orphan_duplicate_crossed_and_funded_proofs() -> TestResult {
    let mut r = ready()?;
    changed_clock(&mut r)?;
    publish(&mut r, Reason::ClockInvalidated)?;
    let good = rows(&r)?;
    let origin = good
        .iter()
        .find(|(k, _)| k.starts_with(history::PREFIX))
        .ok_or("origin")?
        .clone();
    let proof = good
        .iter()
        .find(|(k, _)| k.starts_with(service_proof::PREFIX))
        .ok_or("proof")?
        .clone();
    for docs in [
        vec![origin.clone()],
        vec![proof.clone()],
        vec![origin.clone(), proof.clone(), proof.clone()],
    ] {
        assert!(
            OfferBook::from_documents_with_renewals(
                &r.state.snapshot,
                &docs,
                &r.state.fold.renewal_receipts
            )
            .is_err()
        );
    }
    for (pointer, value) in [
        ("/format", json!(2)),
        ("/operation", json!(token(88))),
        ("/closure/actor", json!(r.offer.terms.buyer)),
        ("/closure/reason", json!("expired")),
        ("/closure/at_ic/clock/generation", json!(1)),
        ("/closure/posted_at_utc", json!(999)),
        ("/closure/action", json!(r.close)),
        ("/closure/commit", json!(token(3))),
    ] {
        let mut v: serde_json::Value = serde_json::from_slice(&proof.1)?;
        *v.pointer_mut(pointer).ok_or("field")? = value;
        let docs = vec![origin.clone(), (proof.0.clone(), serde_json::to_vec(&v)?)];
        assert!(
            OfferBook::from_documents_with_renewals(
                &r.state.snapshot,
                &docs,
                &r.state.fold.renewal_receipts
            )
            .is_err()
        );
    }
    let (mut paid, _, _) = funded(true)?;
    cold(&mut paid)?;
    let mut docs = rows(&paid)?;
    docs.push(proof);
    assert!(
        OfferBook::from_documents_with_renewals(
            &paid.state.snapshot,
            &docs,
            &paid.state.fold.renewal_receipts
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn service_closed_offer_allows_only_fresh_explicit_retry_and_retains_old_proof() -> TestResult {
    for reason in [Reason::DeadlineReached, Reason::ClockInvalidated] {
        let mut r = ready()?;
        match reason {
            Reason::DeadlineReached => r.world.set_now(r.offer.terms.expires_at.at),
            Reason::ClockInvalidated => changed_clock(&mut r)?,
        }
        publish(&mut r, reason)?;
        let old = rows(&r)?;
        r.offer.terms.not_before = r.world.world_ic_now();
        r.offer.terms.expires_at = ClockStamp {
            clock: r.offer.terms.not_before.clock.clone(),
            at: r.offer.terms.not_before.at + 100,
        };
        assert!(install::offer(&r.state, &r.context(false, token(6)), r.offer.clone()).is_err());
        let (_, patch, _) = install::retry(
            &r.state,
            &r.context(false, token(6)),
            r.offer.clone(),
            token(3),
        )?;
        patch.recheck_in(&r.state, &r.context(false, token(6)))?;
        patch.bind(&mut r.state)?.apply();
        for (key, bytes) in old {
            assert!(rows(&r)?.iter().any(|(k, b)| *k == key && *b == bytes));
        }
        cold(&mut r)?;
        assert_eq!(r.state.renewal_audit()?.len(), 2);
        assert!(
            install::retry(
                &r.state,
                &r.context(false, token(7)),
                r.offer.clone(),
                token(3)
            )
            .is_err()
        );
    }
    Ok(())
}

#[test]
fn service_complete_boundary_images_do_not_expand_existing_offer_records() -> TestResult {
    let mut r = Rig::new()?;
    r.six_roles()?;
    r.publish()?;
    let mut origin = r
        .state
        .renewal_offers
        .get(token(3))
        .ok_or("origin")?
        .clone();
    origin.format = 2;
    origin.operation = token(6);
    origin.attempt = Some(RenewalAttemptV1::BuyerRetry {
        previous_operation: token(3),
    });
    let mut low = 0usize;
    let mut high = history::MAX_RECORD_BYTES;
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        let mut candidate = origin.clone();
        candidate.offer.terms.instrument =
            InstrumentId::new(format!("cr{}", "a".repeat(mid))).ok_or("instrument")?;
        candidate.consent_hash = contract(&candidate)?.consent_hash;
        if candidate.validate().is_ok() {
            low = mid
        } else {
            high = mid - 1
        }
    }
    origin.offer.terms.instrument =
        InstrumentId::new(format!("cr{}", "a".repeat(low))).ok_or("instrument")?;
    origin.consent_hash = contract(&origin)?.consent_hash;
    origin.validate()?;
    let before = serde_json::to_vec(&origin)?;
    let mut over = origin.clone();
    over.offer.terms.instrument =
        InstrumentId::new(format!("{}a", origin.offer.terms.instrument)).ok_or("instrument")?;
    over.consent_hash = contract(&over)?.consent_hash;
    assert!(over.validate().is_err(), "actual prior format2 boundary");
    for reason in [Reason::DeadlineReached, Reason::ClockInvalidated] {
        let clock = if reason == Reason::DeadlineReached {
            origin.created_at_ic.clock.clone()
        } else {
            mush_core::ClockRef::WorldIc {
                generation: u64::MAX,
            }
        };
        let closure: RenewalServiceClosureV1 = serde_json::from_value(
            json!({"format":1,"commit":token(7),"action":action(SERVICE_CLOSE_ACTION,1)?,"actor":GOD,"at_ic":ClockStamp{clock,at:i64::MAX},"posted_at_utc":i64::MAX,"reason":reason}),
        )?;
        let record = ServiceRecord {
            format: 1,
            operation: origin.operation,
            closure,
        };
        let terminal = service_proof::contract(&origin, &record)?;
        let writes = vec![
            crate::registry_filing::json_write(
                service_proof::key(origin.operation).into(),
                &record,
            )?,
            crate::registry_filing::json_write(
                crate::api::contract_key(origin.contract),
                &terminal,
            )?,
        ];
        assert!(serde_json::to_vec(&record)?.len() <= service_proof::MAX_BYTES);
        assert!(serde_json::to_vec(&terminal)?.len() <= history::MAX_CONTRACT_BYTES);
        assert!(serde_json::to_vec(&writes)?.len() <= 32768);
        assert_eq!(
            serde_json::from_slice::<Vec<ReplayWrite>>(&serde_json::to_vec(&writes)?)?,
            writes
        );
        assert_eq!(serde_json::to_vec(&origin)?, before);
        assert_eq!(terminal.consent_hash, origin.consent_hash);
    }
    Ok(())
}

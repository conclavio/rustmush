//! Coordinator authority and cold history; inert fixture business data does not
//! claim that Economy has performed a service closure.
use super::*;

fn actor(f: &mut Fixture, actor: ObjId, principal: PrincipalId) -> TestResult {
    let request = RequestUid::from_bytes([90; 16]);
    f.invocation = ActionInvocation::new(ActionInvocationDraft {
        request,
        principal: principal.clone(),
        actor,
        offered_action: ActionOfferId(CanonicalHash::from_bytes([90; 32])),
        subject: PerceivedRef::SelfActor,
        targets: vec![],
        parameters: json!({}),
        idempotency: ScopedIdempotencyKey::new(
            IdempotencyScope {
                principal,
                owner: OwnerId::new("market")?,
                namespace: CanonicalId::new("renewal_fixture")?,
            },
            1,
            IdempotencyKey::Request(request),
        ),
    })?;
    Ok(())
}
fn service_actor(f: &mut Fixture) -> TestResult {
    actor(f, crate::GOD, PrincipalId::Object { id: crate::GOD })
}
fn started(service: bool) -> TestResult<Fixture> {
    let mut f = fixture_config(1, true, service)?;
    let p = draft(&mut f, 31, None)?;
    install(&mut f, p)?;
    Ok(f)
}
fn closure(f: &Fixture, reason: &str) -> TestResult<Value> {
    Ok(json!({"kind":"unfunded_service_closed","closure":{
        "format":1,"commit":CommitToken::from_bytes([32;16]),
        "action":f.service.as_ref().ok_or("service ref")?,"actor":crate::GOD,
        "at_ic":{"clock":{"kind":"world_ic","generation":if reason=="clock_invalidated"{2}else{1}},
            "at":if reason=="clock_invalidated"{0}else{1131}},
        "posted_at_utc":1032,"reason":reason}}))
}
fn close(f: &mut Fixture, state: Value) -> TestResult<PreparedTxn> {
    advance_with_service(f, CommitToken::from_bytes([31; 16]), 5, 32, Some(state))
}

#[test]
fn old_protocol_encoding_and_five_actions_keep_their_exact_identity() -> TestResult {
    let f = fixture()?;
    let protocol = f.store.registry.mind_renewal.as_ref().ok_or("protocol")?;
    assert_eq!(
        serde_json::to_value(protocol)?,
        json!({"manifest":f.manifest,"participants":f.participants,"actions":f.actions})
    );
    let service = fixture_config(1, true, true)?;
    assert_eq!(service.actions, f.actions);
    let new = service
        .store
        .registry
        .mind_renewal
        .as_ref()
        .ok_or("protocol")?;
    let mut encoded = serde_json::to_value(new)?;
    assert_eq!(
        encoded
            .as_object_mut()
            .ok_or("object")?
            .remove("service_close"),
        Some(json!(service.service))
    );
    assert_eq!(encoded, serde_json::to_value(protocol)?);
    assert!(
        Protocol::with_service(
            &OwnerId::new("market")?,
            f.manifest,
            f.participants,
            f.actions.clone(),
            f.actions[4].clone()
        )
        .is_err()
    );
    Ok(())
}

#[test]
fn both_service_causes_consume_seven_targets_without_changing_paid_term() -> TestResult {
    for cause in ["deadline_reached", "clock_invalidated"] {
        let mut f = started(true)?;
        let before = required(&view(&f), &licence_key(LICENCE, 1)?)?;
        let reservation = f
            .store
            .capacity
            .records
            .values()
            .next()
            .ok_or("reservation")?;
        assert_eq!(reservation.record.targets.len(), 7);
        assert_eq!(
            reservation
                .record
                .targets
                .iter()
                .map(|t| t.ceiling)
                .sum::<usize>(),
            81920
        );
        service_actor(&mut f)?;
        let state = closure(&f, cause)?;
        let p = close(&mut f, state)?;
        install(&mut f, p)?;
        assert!(f.store.capacity.is_empty());
        assert_eq!(required(&view(&f), &licence_key(LICENCE, 1)?)?, before);
        assert!(value(&view(&f), &term_key(TERM, 1, 2)?)?.is_none());
        assert!(value(&view(&f), &term_key(ECONOMICS, 1, 2)?)?.is_none());
        hydrate(&f, &f.disk)?;
        let state = closure(&f, cause)?;
        assert!(close(&mut f, state).is_err());
    }
    Ok(())
}

#[test]
fn service_cleanup_requires_exact_registered_extension_and_actual_service_principal() -> TestResult
{
    let mut f = started(false)?;
    service_actor(&mut f)?;
    let state = closure(&f, "deadline_reached")?;
    assert!(close(&mut f, state).is_err());
    let mut f = started(true)?;
    let state = closure(&f, "deadline_reached")?;
    assert!(close(&mut f, state.clone()).is_err());
    actor(&mut f, crate::GOD, PrincipalId::Object { id: ObjId(7) })?;
    assert!(close(&mut f, state.clone()).is_err());
    service_actor(&mut f)?;
    assert!(advance(&mut f, CommitToken::from_bytes([31; 16]), 4, 32).is_err());
    let p = close(&mut f, state)?;
    install(&mut f, p)?;
    Ok(())
}

#[test]
fn service_cannot_close_funded_work_or_create_a_term() -> TestResult {
    let mut f = started(true)?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = advance(&mut f, operation, 1, 32)?;
    install(&mut f, p)?;
    service_actor(&mut f)?;
    let state = closure(&f, "clock_invalidated")?;
    assert!(close(&mut f, state).is_err());
    assert!(advance(&mut f, operation, 2, 33).is_err());
    assert_eq!(pair(&view(&f), operation)?.phase, "funded");
    hydrate(&f, &f.disk)?;
    Ok(())
}

#[test]
fn forged_service_clock_reason_identity_extra_fields_and_receipt_refuse() -> TestResult {
    let mut f = started(true)?;
    service_actor(&mut f)?;
    let good = closure(&f, "deadline_reached")?;
    let mutations = [
        ("/closure/format", json!(2)),
        ("/closure/actor", json!(7)),
        ("/closure/at_ic/at", json!(1130)),
        ("/closure/at_ic/clock/generation", json!(0)),
        ("/closure/reason", json!("clock_invalidated")),
        ("/closure/reason", json!("expired")),
        ("/closure/posted_at_utc", json!(1030)),
        ("/closure/commit", json!(CommitToken::from_bytes([6; 16]))),
        ("/closure/action", json!(f.actions[4])),
    ];
    for (path, value) in mutations {
        let mut bad = good.clone();
        *bad.pointer_mut(path).ok_or("mutation path")? = value;
        assert!(close(&mut f, bad).is_err());
    }
    let mut bad = good.clone();
    bad["closure"]["extra"] = json!(true);
    assert!(close(&mut f, bad).is_err());
    let mut bad = good.clone();
    bad["funding"] = json!({});
    assert!(close(&mut f, bad).is_err());
    let p = close(&mut f, good)?;
    install(&mut f, p)?;
    hydrate(&f, &f.disk)?;
    Ok(())
}

#[test]
fn explicit_buyer_retry_after_service_close_preserves_old_history() -> TestResult {
    for cause in ["deadline_reached", "clock_invalidated"] {
        let mut f = started(true)?;
        let operation = CommitToken::from_bytes([31; 16]);
        service_actor(&mut f)?;
        let state = closure(&f, cause)?;
        let p = close(&mut f, state)?;
        install(&mut f, p)?;
        let old = required(&view(&f), &operation_key(MARKET, operation)?)?;
        actor(&mut f, ObjId(7), PrincipalId::Object { id: ObjId(7) })?;
        assert!(draft(&mut f, 33, None).is_err());
        assert!(draft(&mut f, 30, Some(operation)).is_err());
        let p = draft(&mut f, 33, Some(operation))?;
        install(&mut f, p)?;
        let retry = CommitToken::from_bytes([33; 16]);
        let p = advance(&mut f, retry, 1, 34)?;
        install(&mut f, p)?;
        let p = advance(&mut f, retry, 2, 35)?;
        install(&mut f, p)?;
        assert_eq!(
            required(&view(&f), &operation_key(MARKET, operation)?)?,
            old
        );
        hydrate(&f, &f.disk)?;
    }
    Ok(())
}

#[test]
fn missing_or_forged_service_history_refuses_cold_hydration() -> TestResult {
    let mut f = started(true)?;
    let operation = CommitToken::from_bytes([31; 16]);
    service_actor(&mut f)?;
    let state = closure(&f, "deadline_reached")?;
    let p = close(&mut f, state)?;
    install(&mut f, p)?;
    for family in [MARKET, ROLL, HEAD, RECEIPT] {
        let raw = if family == HEAD {
            licence_key(HEAD, 1)?
        } else {
            operation_key(family, operation)?
        };
        let key = raw.resolve(&f.store.registry)?;
        let mut disk = f.disk.clone();
        assert!(
            disk.remove(&(raw.owner.as_str().into(), key.persistence_key()))
                .is_some()
        );
        assert!(hydrate(&f, &disk).is_err());
    }
    let mut disk = f.disk.clone();
    for family in [MARKET, ROLL, RECEIPT] {
        let raw = operation_key(family, operation)?;
        let key = raw.resolve(&f.store.registry)?;
        let row = disk
            .get_mut(&(raw.owner.as_str().into(), key.persistence_key()))
            .ok_or("row")?;
        row["value"]["state"]["closure"]["reason"] = json!("clock_invalidated");
    }
    assert!(hydrate(&f, &disk).is_err());
    Ok(())
}

#[test]
fn old_five_action_pending_reservation_can_upgrade_and_close_without_rewriting_consent()
-> TestResult {
    let mut old = fixture()?;
    let operation = CommitToken::from_bytes([31; 16]);
    let p = draft(&mut old, 31, None)?;
    install(&mut old, p)?;
    let old_capacity = old
        .store
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation")?
        .canonical
        .clone();
    let mut upgraded = fixture_config(1, true, true)?;
    upgraded.store = hydrate(&upgraded, &old.disk)?;
    upgraded.disk = old.disk;
    assert_eq!(
        upgraded
            .store
            .capacity
            .records
            .values()
            .next()
            .ok_or("reservation")?
            .canonical,
        old_capacity
    );
    let before = pair(&view(&upgraded), operation)?;
    service_actor(&mut upgraded)?;
    let state = closure(&upgraded, "deadline_reached")?;
    let p = close(&mut upgraded, state)?;
    install(&mut upgraded, p)?;
    assert_eq!(
        fingerprint(&pair(&view(&upgraded), operation)?)?,
        fingerprint(&before)?
    );
    hydrate(&upgraded, &upgraded.disk)?;
    Ok(())
}

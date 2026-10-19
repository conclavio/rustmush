//! Real coordinator transactions and canonical hydration with inert business
//! codecs. Roll's genuine worker/filing tests supply certification authority.
use super::*;
const HEADS: &str = super::super::reconformance::HEADS;
fn service() -> Result<Fixture, Box<dyn std::error::Error>> {
    fixture_with_profile(json!({}), true)
}
fn head_key(f: &Fixture) -> Result<String, ActionStateError> {
    Ok(digest(
        "roll.registry.reconformance.head-key.v1",
        &json!(["1", f.manifest.digest()]),
    )?
    .to_string())
}
fn admitted(
    f: &mut Fixture,
    id: CommitToken,
    previous: Option<CommitToken>,
    write_head: bool,
    edit: impl FnOnce(&mut Value, &mut Value, &mut Value),
) -> Result<PreparedTxn, ActionStateError> {
    let key = head_key(f)?;
    let present = f
        .store
        .records
        .keys()
        .any(|k| k.family.as_str() == HEADS && k.encoded.as_str() == key);
    let entry = f.providers.get_exact(&f.actions[0]).unwrap();
    let (mut job, mut execution) = rows(id, 1755, f.manifest.digest());
    let hash = DefinitionDigest::from_bytes([9; 32]);
    execution["admission"]["character"] = json!(crate::GOD);
    execution["admission"]["measured_rejection"] = json!(true);
    execution["admission"]["reconformance"] = json!({"format":1,"filed":"1","birth_rating":{"job":CommitToken::from_bytes([90;16]),"id":hash,"schema":1,"receipt_digest":hash},"previous_attempt":previous});
    let mut head = json!({"format":1,"filed":"1","compatibility":f.manifest.digest(),"job":id,"admitted_at_utc":123});
    edit(&mut job, &mut execution, &mut head);
    let mut txn = f
        .store
        .transaction_for_provider(entry.registration_ordinal())?;
    txn.bind_registered_invocation(entry, &f.invocation, id);
    let base = txn.read_base();
    txn.insert(&f.handles["rating_jobs"], &id.to_hex(), job)?;
    txn.insert(&f.handles["rating_executions"], &id.to_hex(), execution)?;
    if write_head {
        if present {
            txn.replace(&f.handles[HEADS], &key, head)?;
        } else {
            txn.insert(&f.handles[HEADS], &key, head)?;
        }
    }
    let mut context = crate::resolution::PrepareCtx::new(f.invocation.idempotency(), id, None)
        .with_registered_transaction(&base, entry.registration_ordinal());
    context.reserve_registry_rating_completion(&mut txn)?;
    drop(context);
    txn.prepare()
}
fn image(f: &Fixture) -> Result<Vec<ActionStateBootRecord>, Box<dyn std::error::Error>> {
    let mut rows = f
        .store
        .records
        .iter()
        .map(|(k, v)| {
            Ok(ActionStateBootRecord::new(
                k.owner.as_str(),
                k.persistence_key(),
                serde_json::from_slice(v.canonical.as_slice())?,
            ))
        })
        .collect::<Result<Vec<_>, serde_json::Error>>()?;
    rows.extend(
        f.store
            .capacity
            .records
            .values()
            .map(|r| ActionStateBootRecord::new("roll", r.path.as_str(), r.document.clone())),
    );
    Ok(rows)
}
fn cancel(f: &mut Fixture, id: CommitToken, commit: u8) -> TestResult {
    let p = transition(f, id, 3, commit, true, |job, execution| {
        job["state"] = json!({"kind":"cancelled"});
        execution["observation"]["phase"] = json!("cancelled");
    })?;
    install(&mut f.store, p)?;
    Ok(())
}
#[test]
fn service_admission_keeps_exact_targets_old_declaration_bytes_and_cold_head() -> TestResult {
    let old = fixture()?;
    let protocol = old
        .store
        .registry
        .registry_rating
        .as_ref()
        .ok_or("protocol")?;
    assert_eq!(
        serde_json::to_value(protocol)?,
        json!({"manifest":old.manifest,"actions":old.actions})
    );
    let mut f = service()?;
    let id = CommitToken::from_bytes([31; 16]);
    let p = admitted(&mut f, id, None, true, |_, _, _| {})?;
    install(&mut f.store, p)?;
    let reservation = &f
        .store
        .capacity
        .records
        .values()
        .next()
        .ok_or("reservation")?
        .record;
    assert_eq!(reservation.targets, targets(id, 117, true)?);
    assert_eq!(reservation.targets.len(), 123);
    assert_eq!(
        reservation.targets.iter().map(|t| t.ceiling).sum::<usize>(),
        3_981_312
    );
    let disk = image(&f)?;
    service()?.store.hydrate(disk.clone())?;
    assert!(fixture()?.store.hydrate(disk.clone()).is_err());
    for missing in [HEADS, "rating_jobs", "rating_executions"] {
        assert!(
            service()?
                .store
                .hydrate(
                    disk.iter()
                        .filter(|r| !r.key.contains(missing))
                        .cloned()
                        .collect::<Vec<_>>()
                )
                .is_err()
        );
    }
    let mut crossed = disk;
    crossed
        .iter_mut()
        .find(|r| r.key.contains(HEADS))
        .ok_or("head")?
        .value["value"]["job"] = json!(CommitToken::from_bytes([32; 16]));
    assert!(service()?.store.hydrate(crossed).is_err());
    Ok(())
}
#[test]
fn service_binding_and_head_refuse_missing_null_crossed_or_wrong_profile() -> TestResult {
    for fault in 0..11 {
        let mut f = service()?;
        if fault == 9 {
            f.actions = closed_references(&f.manifest)?;
        }
        let result = admitted(
            &mut f,
            CommitToken::from_bytes([33; 16]),
            None,
            fault != 0,
            |_, execution, head| match fault {
                1 => execution["admission"]["reconformance"] = Value::Null,
                2 => {
                    execution["admission"]["reconformance"]
                        .as_object_mut()
                        .unwrap()
                        .remove("previous_attempt");
                }
                3 => execution["admission"]["character"] = json!(7),
                4 => execution["admission"]["measured_rejection"] = json!(false),
                5 => head["job"] = json!(CommitToken::from_bytes([34; 16])),
                6 => head["filed"] = json!("2"),
                7 => execution["admission"]["reconformance"]["filed"] = json!("01"),
                8 => execution["admission"]["reconformance"]["unexpected"] = json!(true),
                10 => head["admitted_at_utc"] = json!(-1),
                _ => {}
            },
        );
        assert!(result.is_err(), "fault {fault} admitted");
        assert!(f.store.capacity.is_empty());
    }
    Ok(())
}
#[test]
fn cancelled_attempt_retry_is_atomic_and_cold_history_cannot_be_pruned_or_branched() -> TestResult {
    let mut f = service()?;
    let first = CommitToken::from_bytes([35; 16]);
    let second = CommitToken::from_bytes([37; 16]);
    let p = admitted(&mut f, first, None, true, |_, _, _| {})?;
    install(&mut f.store, p)?;
    assert!(admitted(&mut f, second, Some(first), true, |_, _, _| {}).is_err());
    cancel(&mut f, first, 36)?;
    assert!(admitted(&mut f, second, None, true, |_, _, _| {}).is_err());
    assert!(
        admitted(
            &mut f,
            second,
            Some(CommitToken::from_bytes([99; 16])),
            true,
            |_, _, _| {}
        )
        .is_err()
    );
    // A real retry's plan includes its new immutable previous-attempt binding;
    // both rows bind that new plan instead of comparing it to the old attempt.
    let p = admitted(&mut f, second, Some(first), true, |job, execution, _| {
        let plan = DefinitionDigest::from_bytes([19; 32]);
        job["plan"] = json!(plan);
        execution["admission"]["plan"] = json!(plan);
    })?;
    install(&mut f.store, p)?;
    service()?.store.hydrate(image(&f)?)?;
    cancel(&mut f, second, 38)?;
    let disk = image(&f)?;
    service()?.store.hydrate(disk.clone())?;
    assert!(
        service()?
            .store
            .hydrate(
                disk.iter()
                    .filter(|r| !r.key.contains(&first.to_hex()))
                    .cloned()
                    .collect::<Vec<_>>()
            )
            .is_err()
    );
    let mut orphan = disk.clone();
    orphan
        .iter_mut()
        .find(|r| r.key.contains(HEADS))
        .ok_or("head")?
        .value["value"]["job"] = json!(first);
    assert!(service()?.store.hydrate(orphan).is_err());
    let mut missing_link = disk;
    missing_link
        .iter_mut()
        .find(|r| r.key.contains("rating_executions") && r.key.contains(&second.to_hex()))
        .ok_or("execution")?
        .value["value"]["admission"]["reconformance"]["previous_attempt"] = Value::Null;
    assert!(service()?.store.hydrate(missing_link).is_err());
    Ok(())
}
#[test]
fn later_transition_cannot_rewrite_the_head_or_borrow_the_ordinary_profile() -> TestResult {
    let mut f = service()?;
    let id = CommitToken::from_bytes([39; 16]);
    let p = admitted(&mut f, id, None, true, |_, _, _| {})?;
    install(&mut f.store, p)?;
    let key = head_key(&f)?;
    let mut head = f.store.get(&f.handles[HEADS], &key)?.ok_or("head")?.clone();
    head["admitted_at_utc"] = json!(124);
    let entry = f.providers.get_exact(&f.actions[1])?;
    let mut txn = f
        .store
        .transaction_for_provider(entry.registration_ordinal())?;
    txn.bind_registered_invocation(entry, &f.invocation, CommitToken::from_bytes([40; 16]));
    txn.replace(&f.handles[HEADS], &key, head)?;
    assert!(txn.prepare().is_err());
    f.actions = closed_references(&f.manifest)?;
    assert!(
        transition(&mut f, id, 3, 40, true, |job, execution| {
            job["state"] = json!({"kind":"cancelled"});
            execution["observation"]["phase"] = json!("cancelled");
        })
        .is_err()
    );
    assert_eq!(f.store.capacity.records.len(), 1);
    Ok(())
}
#[test]
fn changed_manifest_only_recovers_historical_service_work_and_keeps_original_head() -> TestResult {
    let mut old = service()?;
    let id = CommitToken::from_bytes([41; 16]);
    let p = admitted(&mut old, id, None, true, |_, _, _| {})?;
    install(&mut old.store, p)?;
    let mut current = fixture_with_profile(json!({"updated":true}), true)?;
    current.store.hydrate(image(&old)?)?;
    assert!(
        transition(&mut current, id, 3, 42, true, |job, execution| {
            job["state"] = json!({"kind":"cancelled"});
            execution["observation"]["phase"] = json!("cancelled");
        })
        .is_err()
    );
    let p = transition(&mut current, id, 4, 42, true, |job, execution| {
        job["state"] = json!({"kind":"interrupted"});
        execution["observation"]["phase"] = json!("interrupted");
    })?;
    install(&mut current.store, p)?;
    fixture_with_profile(json!({"updated":true}), true)?
        .store
        .hydrate(image(&current)?)?;
    assert!(current.store.capacity.is_empty());
    Ok(())
}

//! Actual final composition audit: corrupt/missing quota never grants recovery
//! authority or turns a retained typed obligation into a healthy empty store.
use super::*;

fn account_key(binding: Binding) -> (String, String) {
    (
        crate::NS.to_string(),
        crate::minds::bulk::publication::account_key(binding.account).to_string(),
    )
}

fn refused_boot(f: &Fixture, disk: &Disk) -> TestResult {
    let before = disk.clone();
    let mut reboot = real_engine(f)?;
    let failure = load(&mut reboot, disk)
        .err()
        .ok_or("inconsistent boot must refuse")?;
    check!(!failure.to_string().is_empty());
    check_eq!(
        disk,
        &before,
        "failed audit cannot repair/drop durable evidence"
    );
    check!(
        reboot
            .world
            .module_state::<RollStore>()
            .ok_or("Roll")?
            .prepare_prune(1)
            .is_err(),
        "the real final-audit gate remains closed"
    );
    command(&mut reboot, "@recovery-pulse");
    command(&mut reboot, "@registry/continue 1");
    check!(
        reboot
            .world
            .module_state::<Artifacts>()
            .ok_or("artifacts")?
            .registry
            .pristine()
    );
    check!(reboot.take_observed_authoritative_submit().is_none());
    Ok(())
}

#[test]
fn final_audit_refuses_missing_all_quota_quarantine_and_crossed_readable_observation() -> TestResult
{
    let mut f = Fixture::new()?;
    let (binding, ready) = f.admitted()?;
    let _ = f.grant(binding, &ready, 4)?;
    drop(ready);
    let key = account_key(binding);
    let original = f.disk.get(&key).ok_or("actual quota")?.clone();
    let mut missing = f.disk.clone();
    missing.remove(&key).ok_or("removed exact actual quota")?;
    refused_boot(&f, &missing)?;

    let mut quarantined = f.disk.clone();
    quarantined.insert(key.clone(), json!({"format":0}));
    refused_boot(&f, &quarantined)?;

    let mut crossed = f.disk.clone();
    let mut account: Account = serde_json::from_value(original)?;
    account
        .runs
        .get_mut(&binding.run)
        .ok_or("run")?
        .charged_unit_ms += 1;
    account.validate(&BulkConfig::default())?;
    crossed.insert(key, serde_json::to_value(account)?);
    refused_boot(&f, &crossed)?;
    Ok(())
}

fn second_admission(f: &mut Fixture) -> TestResult<Binding> {
    let account = AccountId::mint("registry-engine-fixture", 2)?;
    {
        let mut values = f.accounts.values.lock().map_err(module_error)?;
        f.accounts.epoch.invalidate();
        values.insert(GOD, account);
    }
    let source = mush_roll::SourceAdapter::read_source(
        f.engine
            .world
            .module_state::<TacsimStore>()
            .ok_or("sources")?,
        f.revision.source(),
    )?;
    let prepared = f
        .factory
        .prepare(account, GOD, 1, f.revision.clone(), Arc::from(source))?;
    let ticket = f.artifacts_mut()?.registry.admit(prepared.data)?;
    let job = CommitToken::from_bytes([9; 16]);
    let change = f.prepare(Transition::Admit, ticket, job, 9)?;
    f.ack(change, ticket, job, 9, Transition::Admit)?;
    drop(prepared.machine);
    Ok(Binding {
        job,
        account,
        character: GOD,
        run: RunId(1),
    })
}

#[test]
fn final_audit_joins_both_original_accounts_and_refuses_one_missing_private_row() -> TestResult {
    let mut f = Fixture::new()?;
    let (first, ready) = f.admitted()?;
    drop(ready);
    let first_execution: RatingExecution = f.retained("rating_executions")?;
    let second = second_admission(&mut f)?;
    check_ne!(first.account, second.account);
    check!(
        f.quota()?
            .audit_registry_boot(&[first_execution.clone(), first_execution])
            .is_err(),
        "equal counts do not excuse a duplicated typed job"
    );
    let mut complete = real_engine(&f)?;
    load(&mut complete, &f.disk)?;
    let quota = complete.world.module_state::<BulkStore>().ok_or("quota")?;
    check_eq!(quota.stopped_registry_bindings().count(), 2);
    let mut missing = f.disk.clone();
    missing
        .remove(&account_key(first))
        .ok_or("first original account")?;
    refused_boot(&f, &missing)?;
    Ok(())
}

#[test]
fn empty_typed_inventory_still_checks_private_jobs_and_quarantine() -> TestResult {
    let mut f = Fixture::new()?;
    let empty_typed = f.disk.clone();
    let (binding, ready) = f.admitted()?;
    drop(ready);
    // Keep real deployment/source evidence but omit the actual admitted typed
    // job and its roots. Copy only the actual Tacsim owner images from admission.
    let mut orphan = empty_typed.clone();
    for (key, value) in &f.disk {
        if key.0 == crate::NS {
            orphan.insert(key.clone(), value.clone());
        }
    }
    refused_boot(&f, &orphan)?;
    let mut quarantined = empty_typed.clone();
    quarantined.insert(account_key(binding), json!({"format":0}));
    refused_boot(&f, &quarantined)?;
    let mut unreadable = empty_typed;
    unreadable.insert(
        (
            crate::NS.to_string(),
            "tacsim:mindbulk:account:not-an-account".into(),
        ),
        f.disk
            .get(&account_key(binding))
            .ok_or("actual private quota")?
            .clone(),
    );
    refused_boot(&f, &unreadable)?;
    Ok(())
}

#[test]
fn private_boot_join_rejects_duplicate_private_ids_and_typed_inventory_overflow() -> TestResult {
    use mush_roll::commerce::rating::protocol::MAX_STOPPED_REGISTRY_JOBS;
    let mut f = Fixture::new()?;
    let (binding, ready) = f.admitted()?;
    drop(ready);
    let execution: RatingExecution = f.retained("rating_executions")?;
    f.quota()?
        .audit_registry_boot(std::slice::from_ref(&execution))?;
    check!(f.quota()?.audit_registry_boot(&[]).is_err());
    check!(
        f.quota()?
            .audit_registry_boot(&vec![execution.clone(); MAX_STOPPED_REGISTRY_JOBS + 1])
            .is_err()
    );
    // Structural copied account images test only duplicate/bound handling;
    // they do not claim another successful admission or a measured rating.
    let original = f.disk.get(&account_key(binding)).ok_or("quota")?;
    let mut owner = BulkStore::new(BulkConfig::default())?;
    for n in 0..2 {
        let mut account: Account = serde_json::from_value(original.clone())?;
        account.id = AccountId::mint("registry-duplicate-inventory", n)?;
        owner.load(account.id, &serde_json::to_vec(&account)?)?;
    }
    check!(
        owner
            .audit_registry_boot(&[execution.clone(), execution])
            .is_err()
    );
    Ok(())
}

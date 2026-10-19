//! One additional genuine Registry run for the actually saved compatible revision.
use super::*;
fn artifacts(f: &mut PurchaseFixture) -> TestResult<&mut Artifacts> {
    f.engine
        .world
        .module_state_mut::<Artifacts>()
        .ok_or_else(|| "actual artifacts".into())
}
fn quota(f: &PurchaseFixture) -> TestResult<&BulkStore> {
    f.engine
        .world
        .module_state::<BulkStore>()
        .ok_or_else(|| "actual quota".into())
}
fn action(f: &PurchaseFixture, t: Transition) -> TestResult<DefinitionRef> {
    f.engine
        .registry
        .definitions()
        .ok_or("definitions")?
        .iter()
        .map(|e| e.reference())
        .find(|r| {
            r.key().owner.as_str() == "roll"
                && r.key().kind.as_str() == "action"
                && r.key().id.as_str() == t.action_id()
        })
        .cloned()
        .ok_or_else(|| "actual Registry action".into())
}
fn ack(
    f: &mut PurchaseFixture,
    p: PreparedComposedTxn,
    ticket: TicketId,
    job: CommitToken,
    n: u128,
    t: Transition,
) -> TestResult {
    f.install(p)?;
    let a = artifacts(f)?
        .registry
        .take_acknowledged()
        .ok_or("actual Registry acknowledgment")?;
    assert_eq!(
        (a.ticket, a.job, a.operation, a.transition),
        (ticket, job, token(n), t)
    );
    assert!(artifacts(f)?.registry.get(ticket).is_err());
    Ok(())
}
fn typed<T: serde::de::DeserializeOwned>(
    f: &PurchaseFixture,
    family: &str,
    job: CommitToken,
) -> TestResult<T> {
    let value = f
        .rows("roll", family)
        .into_iter()
        .find(|v| {
            v.get("job") == Some(&json!(job)) || v.pointer("/publication/job") == Some(&json!(job))
        })
        .ok_or("actual job row")?;
    Ok(serde_json::from_value(value)?)
}
pub(super) fn certify(f: &mut PurchaseFixture, revision: &Revision) -> TestResult<RatingRef> {
    revision.require_original_access()?;
    let account = crate::minds::bulk::world::account(&f.engine.world, GOD)?;
    let nonce = quota(f)?.next_nonce(account)?;
    let source = mush_roll::SourceAdapter::read_source(
        f.engine
            .world
            .module_state::<TacsimStore>()
            .ok_or("actual source owner")?,
        revision.source(),
    )?;
    let prepared = f
        .factory
        .prepare(account, GOD, nonce, revision.clone(), Arc::from(source))?;
    let start = prepared.data.start.descriptor().sha256;
    let ticket = artifacts(f)?.registry.admit(prepared.data)?;
    let mut n = START + 100;
    let job = token(n);
    let r = revision.reference();
    let p=f.prepare(&action(f,Transition::Admit)?,GOD,n,json!({"ticket":ticket.get(),"mind":r.key.mind,"revision":r.key.number,"revision_digest":r.digest}))?;
    assert!(quota(f)?.active(account).is_none());
    ack(f, p, ticket, job, n, Transition::Admit)?;
    n += 1;
    let binding = Binding {
        account,
        character: GOD,
        run: quota(f)?.active(account).ok_or("actual admitted run")?,
        job,
    };
    let subject: RatingJob = typed(f, "rating_jobs", job)?;
    assert_eq!(subject.subject().revision().revision(), r);
    let mut ready = Ready::registry(binding, start, prepared.machine);
    let executor = Executor::with_wake(&BulkConfig::default(), None)?;
    let deadline = Instant::now() + Duration::from_secs(7200);
    let mut measured = 0u64;
    let mut quanta = 0u64;
    loop {
        if Instant::now() >= deadline {
            return Err("compatible update actual Registry liveness guard".into());
        }
        let ticket = artifacts(f)?
            .registry
            .grant(binding, ready.next_grant_ms())?;
        let p = f.prepare(
            &action(f, Transition::Grant)?,
            GOD,
            n,
            json!({"ticket":ticket.get(),"job":job}),
        )?;
        ack(f, p, ticket, job, n, Transition::Grant)?;
        let lease = f
            .engine
            .world
            .module_state_mut::<BulkStore>()
            .ok_or("quota")?
            .take_registry_lease(token(n))
            .ok_or("actual paid lease")?;
        n += 1;
        executor
            .submit(lease, ready)
            .map_err(|(error, _, _)| error)?;
        let mut done = loop {
            if let Some(done) = executor.poll() {
                break done;
            }
            if Instant::now() >= deadline {
                return Err("compatible update worker liveness guard".into());
            }
            std::thread::sleep(Duration::from_millis(2));
        };
        measured = measured
            .checked_add(done.receipt.actual_unit_ms)
            .ok_or("usage overflow")?;
        let next = done
            .ready
            .take()
            .ok_or("actual Registry Machine retained through ACK")?;
        let settlement = done
            .registry_settlement
            .take()
            .ok_or("actual settlement")??;
        let ticket = artifacts(f)?
            .registry
            .insert_settlement(settlement)
            .map_err(|(error, _)| error)?;
        let before = quota(f)?.run(account, GOD, binding.run)?.clone();
        let p = f.prepare(
            &action(f, Transition::Settle)?,
            GOD,
            n,
            json!({"ticket":ticket.get(),"job":job}),
        )?;
        assert_eq!(quota(f)?.run(account, GOD, binding.run)?, &before);
        ack(f, p, ticket, job, n, Transition::Settle)?;
        n += 1;
        quanta += 1;
        let run = quota(f)?.run(account, GOD, binding.run)?;
        assert_eq!(run.charged_unit_ms, measured);
        if run.status == Status::Complete {
            drop(next);
            break;
        }
        if run.status.terminal() {
            return Err(format!("compatible update Registry terminal {:?}", run.status).into());
        }
        ready = next;
    }
    let receipt: RatingReceipt = typed(f, "rating_receipts", job)?;
    assert_eq!(receipt.subject(), subject.subject());
    assert_eq!(
        receipt.subject().revision().revision(),
        revision.reference()
    );
    assert!(matches!(
        typed::<RatingJob>(f, "rating_jobs", job)?.state(),
        RatingJobState::Completed { .. }
    ));
    eprintln!(
        "M7_STORY phase=compatible_update_certificate event=complete quanta={quanta} unit_ms={measured}"
    );
    Ok(receipt.reference()?)
}

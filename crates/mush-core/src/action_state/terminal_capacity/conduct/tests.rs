//! Inert typed rows test Core affinity and reserved storage only. Actual quota,
//! source, worker measurements and artifact authority belong to native owners.
use super::*;
use crate::{DefinitionDescriptor, DefinitionId, DefinitionKind, DefinitionRegistryBuilder};
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn op(n: u8) -> CommitToken {
    CommitToken::from_bytes([n; 16])
}
fn dg(n: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([n; 32])
}
fn manifest(n: u8) -> TestResult<DefinitionRef> {
    Ok(DefinitionRegistryBuilder::new().register(
        owner("tacsim")?,
        DefinitionDescriptor::new(
            DefinitionKind::new("conduct_manifest")?,
            DefinitionId::new("safe_conduct")?,
            1,
            json!({"fixture":n}),
        )?,
    )?)
}
fn actions(m: &DefinitionRef) -> TestResult<[DefinitionRef; 5]> {
    Ok([
        protocol::action(m, 0)?,
        protocol::action(m, 1)?,
        protocol::action(m, 2)?,
        protocol::action(m, 3)?,
        protocol::action(m, 4)?,
    ])
}
struct Fixture {
    registry: StateFamilyRegistry,
    records: BTreeMap<StateKey, StoredStateRecord>,
    job: wire::Job,
    execution: wire::Execution,
}
impl Fixture {
    fn view(&self) -> View<'_> {
        View {
            registry: &self.registry,
            records: &self.records,
            mutations: None,
        }
    }
    fn put(&mut self, f: &str, id: CommitToken, v: Value) -> TestResult {
        let k = key(f, id)?.resolve(&self.registry)?;
        let d = self
            .registry
            .entries
            .get(&(k.owner.clone(), k.family.clone()))
            .ok_or("descriptor")?
            .descriptor();
        let envelope = json!({"family":{"owner":k.owner,"id":k.family},"key":k.encoded,"schema":{"id":d.value_schema(),"version":1},"value":v});
        self.records.insert(
            k,
            StoredStateRecord {
                value: Box::new(v),
                canonical: CanonicalStateBytes(
                    canonical_json_bytes_with_limit(&envelope, 8192)?.into(),
                ),
            },
        );
        Ok(())
    }
    fn reservation(&self) -> TestResult<Reservation> {
        Ok(Reservation {
            format: 1,
            funding: self.job.id,
            effect: self.job.action.clone(),
            sponsor: key(JOB, self.job.id)?,
            targets: targets(self.job.id)?,
            protocol_fingerprint: Some(self.job.digest()?),
        })
    }
    fn sync(&mut self) -> TestResult {
        self.put(JOB, self.job.id, self.job.raw.clone())?;
        self.put(EXECUTION, self.job.id, self.execution.raw.clone())
    }
}
fn fixture() -> TestResult<Fixture> {
    let m = manifest(1)?;
    let mut builder = StateFamilyRegistryBuilder::new();
    for (family, bytes) in FAMILIES {
        builder.register(
            owner("roll")?,
            protocol::descriptor(family, bytes)?,
            |k: &String| Ok(k.clone()),
            |v: &Value| Ok(v.clone()),
            |_: &Value| Ok(()),
            |v: &Value| Ok(v.clone()),
        )?;
    }
    builder.conduct_trial_capacity(
        &owner("roll")?,
        m.clone(),
        protocol::participants()?,
        actions(&m)?,
    )?;
    let owners = [owner("roll")?, owner("tacsim")?].into_iter().collect();
    let readiness = crate::mechanics::MechanicalOwnerReadiness::for_owners(&owners);
    let registry = builder.freeze_for_owners_with_readiness(&owners, readiness)?;
    let revision = json!({"format":1,"revision":{"fixture":true},"source":dg(1)});
    let worker = json!({"revision":revision,"envelope":dg(2),"schema":dg(3),"definition":dg(4),"expanded":dg(5),"declared":"II","role":"combat"});
    let subject = json!({"charter":"1","line":"2","filed":"3","revision":revision,"rating":{"job":op(9),"id":dg(6),"schema":1,"receipt_digest":dg(7)},"worker":worker,"profile":"safe","role":"escort"});
    let admission = json!({"account_binding":dg(8),"character":crate::GOD,"run":1,"nonce":1,"subject":digest("tacsim.conduct.subject.v1",&json!([worker,"safe","escort"]),4096)?,"policy":dg(10),"schedule":dg(11),"budget":dg(12),"start":{"sha256":dg(13),"bytes":32},"maximum_unit_ms":MAX_CHARGE});
    let job = wire::Job::parse(
        json!({"format":1,"operation":op(1),"action":protocol::action(&m,0)?,"manifest":m,"subject":subject,"admission":admission,"at_utc":1}),
        op(1),
    )?;
    let execution = wire::Execution::parse(
        json!({"format":1,"job":job.id,"updated_by":job.id,"action":job.action,"observation":{"phase":"queued","next_sequence":1,"cursor":{"index":0,"simulated_ms":0},"charged_unit_ms":0,"flight":null,"result":null}}),
        &job,
    )?;
    let mut f = Fixture {
        registry,
        records: BTreeMap::new(),
        job,
        execution,
    };
    f.sync()?;
    Ok(f)
}
fn changed(
    f: &Fixture,
    prior: &wire::Execution,
    index: usize,
    operation: u8,
    edit: impl FnOnce(&mut Value),
) -> TestResult<wire::Execution> {
    let mut v = prior.raw.clone();
    v["updated_by"] = json!(op(operation));
    v["action"] = json!(protocol::action(&f.job.manifest, index)?);
    edit(&mut v["observation"]);
    Ok(wire::Execution::parse(v, &f.job)?)
}
fn granted(f: &Fixture) -> TestResult<wire::Execution> {
    changed(f, &f.execution, 1, 2, |o| {
        o["phase"] = json!("running");
        o["next_sequence"] = json!(2);
        o["charged_unit_ms"] = json!(1200);
        o["flight"] = json!({"sequence":1,"utc_day":20,"allowance_unit_ms":1200,"units":3,"native_ms":100,"cursor":o["cursor"]});
    })
}
fn result(job: &wire::Job, e: &wire::Execution) -> TestResult<Value> {
    Ok(
        json!({"format":1,"job":job.id,"operation":e.operation,"action":e.action,"job_digest":job.digest()?,"execution_digest":e.digest()?,"phase":e.phase,"artifact":e.artifact,"measurement":if e.phase=="complete"{json!({"digest":dg(20),"policy":job.policy,"subject":job.subject,"charged_unit_ms":e.charged,"paired_cases":30,"passed":false})}else{Value::Null}}),
    )
}
#[test]
fn admission_reserves_exact_execution_update_and_future_result() -> TestResult {
    let f = fixture()?;
    let r = f.reservation()?;
    verify(Phase::Funding, &r, &f.view())?;
    assert_eq!(r.targets.len(), 2);
    assert_eq!(r.targets.iter().map(|t| t.ceiling).sum::<usize>(), 8192);
    assert!(matches!(r.targets[0].mode, SlotMode::UpdateExisting));
    assert!(matches!(r.targets[1].mode, SlotMode::Create));
    let mut wrong = r;
    wrong.protocol_fingerprint = Some(dg(31));
    assert!(verify(Phase::Funding, &wrong, &f.view()).is_err());
    Ok(())
}
#[test]
fn full_recipe_and_three_owner_images_are_mandatory() -> TestResult {
    let f = fixture()?;
    let m = f.job.manifest.clone();
    let mut swapped = actions(&m)?;
    swapped.swap(0, 1);
    assert!(
        Protocol::new(
            &owner("roll")?,
            m.clone(),
            protocol::participants()?,
            swapped
        )
        .is_err()
    );
    let mut owners = protocol::participants()?;
    owners.swap(0, 1);
    assert!(Protocol::new(&owner("roll")?, m.clone(), owners, actions(&m)?).is_err());
    let p = f.registry.conduct_trial.as_ref().ok_or("protocol")?;
    for a in actions(&m)? {
        let id = RegisteredTxnIdentity {
            action: a,
            ordinal: 0,
            commit: op(1),
            actor: crate::GOD,
            principal: PrincipalId::Object { id: crate::GOD },
        };
        assert!(
            p.required_images(&id)
                .ok_or("image requirement")?
                .check(&crate::participant::OwnerImages::default())
                .is_err()
        );
    }
    Ok(())
}
#[test]
fn changed_worker_refuses_fresh_work_but_preserves_exact_cleanup() -> TestResult {
    let f = fixture()?;
    let m = manifest(2)?;
    let p = Protocol::new(
        &owner("roll")?,
        m.clone(),
        protocol::participants()?,
        actions(&m)?,
    )?;
    for i in 0..3 {
        assert!(p.current(&f.job, i).is_err());
        assert!(protocol::action(&m, i)? != protocol::action(&f.job.manifest, i)?);
    }
    for i in 3..5 {
        p.current(&f.job, i)?;
        assert!(protocol::action(&m, i)? == protocol::action(&f.job.manifest, i)?);
    }
    Ok(())
}
#[test]
fn required_nullable_fields_and_closed_shapes_refuse_omission() -> TestResult {
    let f = fixture()?;
    for n in ["flight", "result"] {
        let mut v = f.execution.raw.clone();
        v["observation"]
            .as_object_mut()
            .ok_or("observation")?
            .remove(n);
        assert!(wire::Execution::parse(v, &f.job).is_err());
    }
    let mut v = f.job.raw.clone();
    v["admission"]["authority"] = json!(true);
    assert!(wire::Job::parse(v, op(1)).is_err());
    let mut v = f.job.raw.clone();
    v["subject"]["charter"] = json!("01");
    assert!(wire::Job::parse(v, op(1)).is_err());
    let mut v = f.job.raw.clone();
    v["admission"]["maximum_unit_ms"] = json!(MAX_CHARGE + 1);
    assert!(wire::Job::parse(v, op(1)).is_err());
    Ok(())
}
#[test]
fn grant_and_settlement_conserve_prepaid_allowance_and_cursor() -> TestResult {
    let f = fixture()?;
    let g = granted(&f)?;
    transition::progression(&f.execution, &g, 1)?;
    let settled = changed(&f, &g, 2, 3, |o| {
        o["flight"] = Value::Null;
        o["charged_unit_ms"] = json!(300);
        o["cursor"]["simulated_ms"] = json!(100);
    })?;
    transition::progression(&g, &settled, 2)?;
    let excess = changed(&f, &g, 2, 3, |o| {
        o["flight"] = Value::Null;
        o["charged_unit_ms"] = json!(1201);
    })?;
    assert!(transition::progression(&g, &excess, 2).is_err());
    let skipped = changed(&f, &g, 2, 3, |o| {
        o["flight"] = Value::Null;
        o["cursor"]["index"] = json!(2);
    })?;
    assert!(transition::progression(&g, &skipped, 2).is_err());
    assert!(transition::progression(&g, &g, 1).is_err());
    Ok(())
}
#[test]
fn cancellation_retains_inflight_accounting_and_wins_settlement() -> TestResult {
    let f = fixture()?;
    let g = granted(&f)?;
    let cancel = changed(&f, &g, 3, 3, |o| o["phase"] = json!("cancel_requested"))?;
    transition::progression(&g, &cancel, 3)?;
    let repeated = changed(&f, &cancel, 3, 4, |_| {})?;
    assert!(transition::progression(&cancel, &repeated, 3).is_err());
    let settled = changed(&f, &cancel, 2, 4, |o| {
        o["phase"] = json!("cancelled");
        o["flight"] = Value::Null;
    })?;
    transition::progression(&cancel, &settled, 2)?;
    let escape = changed(&f, &cancel, 2, 4, |o| {
        o["phase"] = json!("running");
        o["flight"] = Value::Null;
    })?;
    assert!(transition::progression(&cancel, &escape, 2).is_err());
    Ok(())
}
#[test]
fn recovery_preserves_charged_work_without_fabricated_measurement() -> TestResult {
    let f = fixture()?;
    let g = granted(&f)?;
    let end = changed(&f, &g, 4, 3, |o| {
        o["phase"] = json!("interrupted");
        o["flight"] = Value::Null;
    })?;
    transition::progression(&g, &end, 4)?;
    wire::result(&result(&f.job, &end)?, &f.job, &end)?;
    let refund = changed(&f, &g, 4, 3, |o| {
        o["phase"] = json!("interrupted");
        o["flight"] = Value::Null;
        o["charged_unit_ms"] = json!(0);
    })?;
    assert!(transition::progression(&g, &refund, 4).is_err());
    let mut invented = result(&f.job, &end)?;
    invented["measurement"] = json!({"passed":true});
    assert!(wire::result(&invented, &f.job, &end).is_err());
    Ok(())
}
#[test]
fn completed_failed_conduct_is_retained_but_crossed_measurement_refuses() -> TestResult {
    let f = fixture()?;
    let g = granted(&f)?;
    let e = changed(&f, &g, 2, 3, |o| {
        o["phase"] = json!("complete");
        o["flight"] = Value::Null;
        o["cursor"] = json!({"index":59,"simulated_ms":120000});
        o["result"] = json!({"sha256":dg(21),"bytes":200});
    })?;
    let mut r = result(&f.job, &e)?;
    wire::result(&r, &f.job, &e)?;
    r["measurement"]["subject"] = json!(dg(22));
    assert!(wire::result(&r, &f.job, &e).is_err());
    let mut r = result(&f.job, &e)?;
    r["execution_digest"] = json!(dg(23));
    assert!(wire::result(&r, &f.job, &e).is_err());
    Ok(())
}
#[test]
fn cold_inventory_rejects_missing_job_execution_or_reservation_and_orphans() -> TestResult {
    let mut f = fixture()?;
    let saved = Arc::new(encode(f.reservation()?)?);
    let reservations = BTreeMap::from([(saved.id(), saved)]);
    audit_boot(&f.view(), &reservations)?;
    assert!(audit_boot(&f.view(), &BTreeMap::new()).is_err());
    for family in [JOB, EXECUTION] {
        let k = key(family, op(1))?.resolve(&f.registry)?;
        let v = f.records.remove(&k).ok_or("record")?;
        assert!(audit_boot(&f.view(), &reservations).is_err());
        f.records.insert(k, v);
    }
    f.put(RESULT, op(7), json!({}))?;
    assert!(audit_boot(&f.view(), &reservations).is_err());
    Ok(())
}
#[test]
fn actual_terminal_pair_consumes_capacity_and_forbids_repeated_completion() -> TestResult {
    let mut f = fixture()?;
    let reservation = f.reservation()?;
    f.execution = changed(&f, &f.execution, 3, 2, |o| o["phase"] = json!("cancelled"))?;
    f.sync()?;
    assert!(verify(Phase::Terminal, &reservation, &f.view()).is_err());
    f.put(RESULT, op(1), result(&f.job, &f.execution)?)?;
    verify(Phase::Terminal, &reservation, &f.view())?;
    audit_boot(&f.view(), &BTreeMap::new())?;
    let saved = Arc::new(encode(reservation)?);
    assert!(audit_boot(&f.view(), &BTreeMap::from([(saved.id(), saved)])).is_err());
    let later = changed(&f, &f.execution, 4, 3, |o| {
        o["phase"] = json!("interrupted")
    })?;
    assert!(transition::progression(&f.execution, &later, 4).is_err());
    Ok(())
}

use super::*;
use mush_core::ClockRef;
use serde_json::json;
type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
fn hash(byte: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([byte; 32])
}
fn subject() -> TestResult<RatingSubject> {
    let revision = serde_json::from_value(
        json!({"format":1,"revision":{"key":{"character":7,"mind":1,"number":1},"digest":hash(1)},"source":hash(2)}),
    )?;
    Ok(RatingSubject::new(
        revision,
        hash(3),
        hash(4),
        hash(5),
        hash(6),
        RatingClass::I,
    ))
}
fn budget_row(class: RatingClass) -> TestResult<CaseResult> {
    Ok(CaseResult::new(
        CaseKey::new(
            class,
            CaseKind::Probe {
                pair: 0,
                name: ProbeName::EmptyArmed,
            },
        )?,
        CaseBinding::new(hash(2), hash(5), hash(7), hash(8)),
        MeterSummary::new(1, 100, 0, Some(Peak::new(100, 0, 0, 1)?), 1, 0, 0, 0)?,
        Coverage::new(Applicability::Armed, 1, 0, hash(9))?,
        0,
        CaseOutcome::BudgetExceeded {},
        None,
    )?)
}
fn evidence(classes: &[RatingClass]) -> TestResult<MeasuredNonconformance> {
    let subject = subject()?;
    let compatibility = RatingCompatibility::combat_v1(hash(10));
    let rows = classes
        .iter()
        .copied()
        .map(budget_row)
        .collect::<TestResult<Vec<_>>>()?;
    let page = PageContent::new(subject.digest()?, compatibility.digest(), 0, rows)?;
    Ok(MeasuredNonconformance::maximum_class_budget(
        subject,
        compatibility,
        hash(11),
        vec![page],
    )?)
}
#[test]
fn measured_rejection_requires_every_admitted_class_through_the_maximum() -> TestResult {
    let row = evidence(&[RatingClass::I, RatingClass::II, RatingClass::III])?;
    assert_eq!(row.cause(), NonconformanceCause::MaximumClassBudget);
    for classes in [
        vec![RatingClass::I],
        vec![RatingClass::I, RatingClass::II],
        vec![RatingClass::I, RatingClass::III],
        vec![RatingClass::II, RatingClass::III],
        vec![RatingClass::III],
    ] {
        assert!(evidence(&classes).is_err());
    }
    let mut only_three = row.clone();
    only_three.subject.declared = RatingClass::III;
    only_three.pages.0[0].rows.0 = vec![budget_row(RatingClass::III)?];
    only_three.pages.0[0].subject = only_three.subject.digest()?;
    only_three.validate()?;
    Ok(())
}
#[test]
fn rejection_does_not_accept_a_passing_case_or_infrastructure_fault() -> TestResult {
    let row = evidence(&[RatingClass::I, RatingClass::II, RatingClass::III])?;
    let last = row.pages.0[0].rows.0.last().ok_or("last row")?;
    let mut clean = last.clone();
    clean.meter = MeterSummary::new(1, 100, 0, Some(Peak::new(100, 0, 0, 1)?), 0, 0, 0, 0)?;
    clean.coverage = Coverage::new(Applicability::Armed, 1, 1, hash(9))?;
    clean.outcome = CaseOutcome::Conformant {};
    clean.validate()?;
    let mut changed = row.clone();
    *changed.pages.0[0].rows.0.last_mut().ok_or("last row")? = clean;
    assert!(changed.validate().is_err());
    for field in ["other_faults", "unmeasured", "dropped"] {
        let mut wire = serde_json::to_value(&row)?;
        wire["pages"][0]["rows"][2]["meter"][field] = json!(1);
        assert!(serde_json::from_value::<MeasuredNonconformance>(wire).is_err());
    }
    Ok(())
}
#[test]
fn rejected_artifact_strictly_binds_source_manifest_order_and_pages() -> TestResult {
    let row = evidence(&[RatingClass::I, RatingClass::II, RatingClass::III])?;
    for path in [vec!["subject"], vec!["compatibility"]] {
        let mut wire = serde_json::to_value(&row)?;
        wire["pages"][0][path[0]] = json!(hash(99));
        assert!(serde_json::from_value::<MeasuredNonconformance>(wire).is_err());
    }
    let mut wrong = row.clone();
    wrong.pages.0[0].rows.0[2].binding.source = hash(99);
    assert!(wrong.validate().is_err());
    let mut wrong = row.clone();
    wrong.pages.0[0].rows.0.swap(0, 1);
    assert!(wrong.validate().is_err());
    let mut wrong = row.clone();
    wrong.pages.0[0].ordinal = 1;
    assert!(wrong.validate().is_err());
    let mut wrong = row.clone();
    let tail = wrong.pages.0[0].rows.0.split_off(2);
    wrong.pages.0.push(PageContent::new(
        row.subject.digest()?,
        row.compatibility.digest(),
        1,
        tail,
    )?);
    assert!(
        wrong.validate().is_err(),
        "short nonfinal page is not canonical"
    );
    let mut wire = serde_json::to_value(&row)?;
    wire["cancelled"] = json!(true);
    assert!(serde_json::from_value::<MeasuredNonconformance>(wire).is_err());
    let mut wire = serde_json::to_value(&row)?;
    wire["format"] = json!(2);
    assert!(serde_json::from_value::<MeasuredNonconformance>(wire).is_err());
    Ok(())
}
#[test]
fn result_union_preserves_existing_success_bytes_and_never_certifies_rejection() -> TestResult {
    let complete = super::super::test_support::complete_rating(
        subject()?,
        RatingCompatibility::combat_v1(hash(10)),
        hash(11),
        CommitToken::from_bytes([12; 16]),
        ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 10,
        },
    )?;
    let cert = execution::MeasuredRating::from_receipt(&complete.receipt, &complete.pages)?;
    let old = serde_json::to_vec(&cert)?;
    let result = MeasuredRegistryResult::Certified(cert.clone());
    assert_eq!(serde_json::to_vec(&result)?, old);
    assert_eq!(result.artifact()?, cert.artifact()?);
    assert_eq!(
        serde_json::from_slice::<MeasuredRegistryResult>(&old)?,
        result
    );
    let rejected = evidence(&[RatingClass::I, RatingClass::II, RatingClass::III])?;
    let bytes = serde_json::to_vec(&rejected)?;
    assert!(serde_json::from_slice::<execution::MeasuredRating>(&bytes).is_err());
    assert_eq!(
        serde_json::from_slice::<MeasuredRegistryResult>(&bytes)?,
        MeasuredRegistryResult::Nonconforming(rejected.clone())
    );
    assert_eq!(rejected.artifact()?.bytes, bytes.len() as u64);
    assert_eq!(
        rejected.artifact()?.sha256,
        DefinitionDigest::from_bytes(Sha256::digest(&bytes).into())
    );
    let mut crossed = serde_json::to_value(&rejected)?;
    crossed["content"] = serde_json::to_value(&cert)?["content"].clone();
    assert!(serde_json::from_value::<MeasuredRegistryResult>(crossed).is_err());
    Ok(())
}

#[test]
fn rejection_header_reconstructs_exact_worker_bytes_and_cannot_decode_as_certificate() -> TestResult
{
    let measured = evidence(&[RatingClass::I, RatingClass::II, RatingClass::III])?;
    let artifact = measured.artifact()?;
    let publication = RatingPublication::new(
        CommitToken::from_bytes([3; 16]),
        mush_core::ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
    )?;
    let (receipt, pages) = measured.publish_parts(publication)?;
    assert_eq!(
        MeasuredNonconformance::from_receipt(&receipt, &pages)?.artifact()?,
        artifact
    );
    let bytes = serde_json::to_vec(&receipt)?;
    assert_eq!(
        serde_json::from_slice::<RatingNonconformance>(&bytes)?,
        receipt
    );
    assert!(serde_json::from_slice::<RatingReceipt>(&bytes).is_err());
    assert!(receipt.verify_pages(&[]).is_err());
    let mut crossed = pages.clone();
    crossed[0].job = CommitToken::from_bytes([9; 16]);
    assert!(receipt.verify_pages(&crossed).is_err());
    let mut crossed = pages;
    crossed[0].content.rows.0[0].binding.source = hash(99);
    assert!(receipt.verify_pages(&crossed).is_err());
    let mut wrong = serde_json::to_value(&receipt)?;
    wrong["publication"]["job"] = json!(CommitToken::from_bytes([9; 16]));
    let other: RatingNonconformance = serde_json::from_value(wrong)?;
    assert_ne!(other.reference()?, receipt.reference()?);
    Ok(())
}

#[test]
fn rejected_execution_requires_new_admission_and_the_exact_artifact() -> TestResult {
    use crate::commerce::rating::execution::*;
    let measured = evidence(&[RatingClass::I, RatingClass::II, RatingClass::III])?;
    let artifact = measured.artifact()?;
    let publication = RatingPublication::new(
        CommitToken::from_bytes([3; 16]),
        mush_core::ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
    )?;
    let (receipt, pages) = measured.publish_parts(publication)?;
    let queued = RatingJob::new(
        receipt.publication().job(),
        receipt.subject().clone(),
        receipt.compatibility().clone(),
        receipt.plan(),
    );
    let running = queued.transition(RatingJobState::Running { lease_sequence: 1 })?;
    let terminal = running.transition(RatingJobState::Nonconforming {
        receipt: receipt.reference()?,
    })?;
    let admission = ExecutionAdmission {
        job_account_binding: hash(12),
        character: mush_core::ObjId(7),
        run: 1,
        nonce: 1,
        subject: receipt.subject().digest()?,
        compatibility: receipt.compatibility().digest(),
        plan: receipt.plan(),
        budget: hash(13),
        start: ArtifactIdentity {
            sha256: hash(14),
            bytes: 100,
        },
        maximum_unit_ms: 10000,
        cases: 3,
        maximum_pages: 1,
        measured_rejection: true,
    };
    let execution: RatingExecution = serde_json::from_value(
        json!({"format":1,"job":queued.job(),"admission":admission,
        "observation":{"phase":"complete","next_sequence":2,"cursor":{"index":2,"simulated_ms":250},"charged_unit_ms":750,"flight":null,"result":artifact},
        "updated_by":CommitToken::from_bytes([4;16])}),
    )?;
    execution.verify_nonconformance(&terminal, &receipt, &pages)?;
    assert_eq!(crate::commerce::rating_roots::required(&terminal)?.len(), 1);
    assert!(terminal.transition(RatingJobState::Cancelled {}).is_err());
    let mut old = serde_json::to_value(&execution)?;
    old["admission"]
        .as_object_mut()
        .ok_or("admission")?
        .remove("measured_rejection");
    let old: RatingExecution = serde_json::from_value(old)?;
    assert!(
        old.verify_nonconformance(&terminal, &receipt, &pages)
            .is_err()
    );
    assert!(
        !serde_json::to_value(&old)?["admission"]
            .as_object()
            .ok_or("admission")?
            .contains_key("measured_rejection")
    );
    let mut crossed = serde_json::to_value(&execution)?;
    crossed["observation"]["result"]["sha256"] = json!(hash(99));
    let crossed: RatingExecution = serde_json::from_value(crossed)?;
    assert!(
        crossed
            .verify_nonconformance(&terminal, &receipt, &pages)
            .is_err()
    );
    Ok(())
}

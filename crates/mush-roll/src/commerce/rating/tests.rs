use super::*;
use mush_core::ClockRef;
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;
fn hash(byte: u8) -> DefinitionDigest {
    DefinitionDigest::from_bytes([byte; 32])
}
fn subject() -> Result<RatingSubject, serde_json::Error> {
    let revision: RevisionIdentity = serde_json::from_value(
        json!({"format":1,"revision":{"key":{"character":7,"mind":1,"number":1},"digest":hash(1)},"source":hash(2)}),
    )?;
    Ok(RatingSubject::new(
        revision,
        hash(3),
        hash(9),
        hash(4),
        hash(5),
        RatingClass::I,
    ))
}
fn row(class: RatingClass, kind: CaseKind, ops: u64, fees: u64) -> Result<CaseResult, RatingError> {
    let kernel = matches!(kind, CaseKind::Kernel { .. });
    CaseResult::new(
        CaseKey::new(class, kind)?,
        CaseBinding::new(hash(2), hash(4), hash(6), hash(7)),
        MeterSummary::new(1, ops, fees, Some(Peak::new(ops, fees, 0, 0)?), 0, 0, 0, 0)?,
        Coverage::new(
            if kernel {
                Applicability::EngineBoundary
            } else {
                Applicability::Armed
            },
            1,
            1,
            hash(8),
        )?,
        0,
        if kernel {
            CaseOutcome::Kernel {
                expected: KernelOutcome::Passed,
                actual: KernelOutcome::Passed,
            }
        } else {
            CaseOutcome::Conformant {}
        },
        matches!(kind, CaseKind::Trial { .. })
            .then(|| TrialEnd::new(1000, 42, hash(9), TrialOutcome::Timeout))
            .transpose()?,
    )
}
fn certificate() -> Result<(RatingReceipt, Vec<RatingPage>), Box<dyn std::error::Error>> {
    let subject = subject()?;
    let compatibility = RatingCompatibility::combat_v1(hash(10));
    let rows = vec![row(
        RatingClass::I,
        CaseKind::Probe {
            pair: 0,
            name: ProbeName::EmptyArmed,
        },
        100,
        20,
    )?];
    let summary = ClassSummary::from_cases(RatingClass::I, &rows.iter().collect::<Vec<_>>())?;
    let content = PageContent::new(subject.digest()?, compatibility.digest(), 0, rows)?;
    let result = RatingResultRef::from_pages(std::slice::from_ref(&content))?;
    let cert = CertificateContent::new(
        subject,
        compatibility,
        hash(20),
        vec![VariantAlias::new(AliasKind::Profile, "Line".into(), 0)?],
        vec![summary],
        RatingClass::I,
        result.clone(),
    )?;
    let receipt = RatingReceipt::new(
        cert,
        RatingPublication::new(
            CommitToken::from_bytes([1; 16]),
            ClockStamp {
                clock: ClockRef::WorldIc { generation: 1 },
                at: 100,
            },
        )?,
    )?;
    let page = RatingPage::new(
        receipt.publication().job(),
        receipt.id(),
        result.id(),
        content,
    )?;
    Ok((receipt, vec![page]))
}

#[test]
fn strict_ids_have_no_numeric_or_uppercase_alias() -> TestResult {
    let text = hash(0xab).to_string();
    assert_eq!(RatingId::from_hex(&text)?.to_string(), text);
    for bad in [
        text.to_uppercase(),
        text.trim_start_matches('a').to_owned(),
        "0".repeat(65),
    ] {
        assert!(RatingId::from_hex(&bad).is_err());
    }
    assert!(serde_json::from_str::<RatingId>("12").is_err());
    Ok(())
}
#[test]
fn exact_receipt_pages_roundtrip_and_bind_parent_key() -> TestResult {
    let (receipt, pages) = certificate()?;
    receipt.verify_pages(&pages)?;
    let bytes = encoded(&receipt, MAX_RECEIPT_BYTES)?;
    assert_eq!(
        decode_receipt(&receipt.publication().job().to_string(), &bytes)?,
        receipt
    );
    let page = pages.first().ok_or("page")?;
    assert_eq!(
        decode_page(&page.key().encoded(), &encoded(page, MAX_PAGE_BYTES)?)?,
        *page
    );
    assert!(decode_receipt(&hash(9).to_string(), &bytes).is_err());
    let mut wrong = pages.clone();
    wrong.first_mut().ok_or("page")?.rating = RatingId(hash(9));
    assert!(receipt.verify_pages(&wrong).is_err());
    Ok(())
}
#[test]
fn publication_is_outside_content_id_but_inside_full_reference() -> TestResult {
    let (receipt, _) = certificate()?;
    let mut later = receipt.clone();
    later.publication.issued.at += 1;
    later.validate()?;
    assert_eq!(receipt.id(), later.id());
    assert_ne!(receipt.reference()?, later.reference()?);
    Ok(())
}

#[test]
fn identical_measurements_from_distinct_jobs_have_distinct_durable_keys() -> TestResult {
    let (first, first_pages) = certificate()?;
    let job = CommitToken::from_bytes([2; 16]);
    let second = RatingReceipt::new(
        first.content.clone(),
        RatingPublication::new(job, first.publication.issued.clone())?,
    )?;
    let second_pages = first_pages
        .iter()
        .map(|page| RatingPage::new(job, second.id(), second.result().id(), page.content.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    assert_eq!(first.id(), second.id(), "attempt is not measured content");
    assert_eq!(first.result(), second.result());
    assert_ne!(first.reference()?, second.reference()?);
    assert_eq!(second.reference()?.job(), job);
    assert_ne!(
        first_pages.first().ok_or("first page")?.key(),
        second_pages.first().ok_or("second page")?.key(),
    );
    first.verify_pages(&first_pages)?;
    second.verify_pages(&second_pages)?;
    assert!(first.verify_pages(&second_pages).is_err());
    assert!(second.verify_pages(&first_pages).is_err());
    let bytes = encoded(&first, MAX_RECEIPT_BYTES)?;
    assert!(decode_receipt(&job.to_string(), &bytes).is_err());
    assert!(
        decode_receipt(&first.id().to_string(), &bytes).is_err(),
        "the old content-only key cannot alias a job"
    );
    Ok(())
}

#[test]
fn references_and_pages_require_exact_attempt_identity() -> TestResult {
    let (receipt, pages) = certificate()?;
    let reference = receipt.reference()?;
    let mut missing = serde_json::to_value(reference)?;
    missing.as_object_mut().ok_or("reference")?.remove("job");
    assert!(serde_json::from_value::<RatingRef>(missing).is_err());
    let mut missing = serde_json::to_value(pages.first().ok_or("page")?)?;
    missing.as_object_mut().ok_or("page")?.remove("job");
    assert!(serde_json::from_value::<RatingPage>(missing).is_err());
    for job in [json!(1), json!("A".repeat(32)), json!("0".repeat(31))] {
        let mut malformed = serde_json::to_value(reference)?;
        malformed
            .as_object_mut()
            .ok_or("reference")?
            .insert("job".into(), job);
        assert!(serde_json::from_value::<RatingRef>(malformed).is_err());
    }
    Ok(())
}
#[test]
fn missing_duplicate_unknown_and_nullable_fields_are_rejected() -> TestResult {
    let (receipt, pages) = certificate()?;
    let bytes = String::from_utf8(encoded(&receipt, MAX_RECEIPT_BYTES)?)?;
    let duplicate = bytes.replacen("\"format\":1", "\"format\":1,\"format\":1", 1);
    assert!(serde_json::from_str::<RatingReceipt>(&duplicate).is_err());
    let mut value = serde_json::to_value(&receipt)?;
    value
        .as_object_mut()
        .ok_or("receipt")?
        .insert("actor".into(), json!(9));
    assert!(serde_json::from_value::<RatingReceipt>(value).is_err());
    let mut value = serde_json::to_value(
        pages
            .first()
            .ok_or("page")?
            .content
            .rows()
            .first()
            .ok_or("row")?,
    )?;
    value.as_object_mut().ok_or("row")?.remove("terminal");
    assert!(serde_json::from_value::<CaseResult>(value).is_err());
    for text in [
        r#"{"kind":"profile_cycle","actor":7}"#,
        r#"{"kind":"conformant","source":"private"}"#,
    ] {
        if text.contains("profile_cycle") {
            assert!(serde_json::from_str::<CaseKind>(text).is_err());
        } else {
            assert!(serde_json::from_str::<CaseOutcome>(text).is_err());
        }
    }
    Ok(())
}
#[test]
fn every_page_and_result_reference_is_required() -> TestResult {
    let (receipt, pages) = certificate()?;
    assert!(receipt.verify_pages(&[]).is_err());
    let mut wrong = pages.clone();
    wrong.first_mut().ok_or("page")?.content.subject = hash(99);
    assert!(receipt.verify_pages(&wrong).is_err());
    let mut wrong = pages.clone();
    wrong
        .first_mut()
        .ok_or("page")?
        .content
        .rows
        .0
        .first_mut()
        .ok_or("row")?
        .meter
        .max_ops += 1;
    assert!(receipt.verify_pages(&wrong).is_err());
    let mut bad = serde_json::to_value(receipt.result())?;
    bad.get_mut("pages")
        .and_then(Value::as_array_mut)
        .ok_or("pages")?
        .first_mut()
        .ok_or("page")?
        .as_object_mut()
        .ok_or("reference")?
        .insert("ordinal".into(), json!(1));
    assert!(serde_json::from_value::<RatingResultRef>(bad).is_err());
    Ok(())
}
#[test]
fn class_peak_is_an_actual_invocation_not_sum_of_unrelated_maxima() -> TestResult {
    let rows = [
        row(
            RatingClass::I,
            CaseKind::Probe {
                pair: 0,
                name: ProbeName::EmptyArmed,
            },
            100,
            1,
        )?,
        row(
            RatingClass::I,
            CaseKind::Probe {
                pair: 0,
                name: ProbeName::EmptyUnarmed,
            },
            1,
            99,
        )?,
    ];
    let summary = ClassSummary::from_cases(RatingClass::I, &rows.iter().collect::<Vec<_>>())?;
    assert_eq!(
        (summary.meter.max_ops(), summary.meter.max_fees()),
        (100, 99)
    );
    assert_eq!(summary.meter.peak().ok_or("peak")?.total()?, 101);
    assert_eq!(summary.peak_case, rows.first().ok_or("first")?.key);
    assert!(Peak::new(u64::MAX, 1, 0, 0).is_err());
    assert!(MeterSummary::new(1, 10, 10, Some(Peak::new(1, 1, 0, 0)?), 0, 0, 0, 0).is_err());
    Ok(())
}
#[test]
fn aliases_classes_and_declared_minimum_cannot_be_relabelled() -> TestResult {
    let (receipt, _) = certificate()?;
    let mut content = receipt.content.clone();
    content.minimum = RatingClass::II;
    assert!(content.validate().is_err());
    let mut content = receipt.content.clone();
    content.classes.0.first_mut().ok_or("class")?.class = RatingClass::II;
    assert!(content.validate().is_err());
    let mut content = receipt.content.clone();
    content
        .aliases
        .0
        .push(content.aliases.0.first().ok_or("alias")?.clone());
    assert!(content.validate().is_err());
    Ok(())
}
#[test]
fn full_representational_case_bound_fits_complete_page_envelopes() -> TestResult {
    let mut rows = Vec::new();
    let probes = [
        ProbeName::EmptyArmed,
        ProbeName::EmptyUnarmed,
        ProbeName::Hostile450Armed,
        ProbeName::Hostile450Unarmed,
        ProbeName::HurtTwoArmed,
        ProbeName::HurtTwoUnarmed,
        ProbeName::OrderLossArmed,
        ProbeName::OrderLossUnarmed,
        ProbeName::DensityArmed,
        ProbeName::DensityUnarmed,
        ProbeName::FordMobile,
        ProbeName::FordDisabled,
    ];
    let kernels = [
        KernelName::MemoryKeysAt,
        KernelName::MemoryKeysOver,
        KernelName::MemoryValueAt,
        KernelName::MemoryValueOver,
        KernelName::RulesSlotsAt,
        KernelName::RulesSlotsOver,
        KernelName::MeterFinalFee,
        KernelName::MeterCallback,
    ];
    for class in [RatingClass::I, RatingClass::II, RatingClass::III] {
        for variant in 0..12 {
            for layout in [TrialLayout::Duel, TrialLayout::Muster, TrialLayout::Hold] {
                for seed in 1..=2 {
                    for swapped in [false, true] {
                        rows.push(row(
                            class,
                            CaseKind::Trial {
                                variant,
                                layout,
                                seed,
                                swapped,
                            },
                            u64::MAX - 1,
                            1,
                        )?);
                    }
                }
            }
        }
        for pair in 0..36 {
            for name in probes {
                rows.push(row(class, CaseKind::Probe { pair, name }, u64::MAX - 1, 1)?);
            }
        }
        rows.push(row(class, CaseKind::ProfileCycle {}, u64::MAX - 1, 1)?);
        for name in kernels {
            rows.push(row(class, CaseKind::Kernel { name }, u64::MAX - 1, 1)?);
        }
    }
    assert_eq!(rows.len(), MAX_CASES);
    let contents = rows
        .chunks(MAX_PAGE_ROWS)
        .enumerate()
        .map(|(i, rows)| {
            PageContent::new(
                hash(1),
                hash(2),
                u16::try_from(i).map_err(|_| RatingError::Bound)?,
                rows.to_vec(),
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    let result = RatingResultRef::from_pages(&contents)?;
    assert_eq!(result.pages().len(), MAX_PAGES);
    let mut total = 0;
    for content in contents {
        let page = RatingPage::new(
            CommitToken::from_bytes([3; 16]),
            RatingId(hash(3)),
            result.id(),
            content,
        )?;
        let size = encoded(&page, MAX_PAGE_BYTES)?.len() + TYPED_ENVELOPE_RESERVE;
        assert!(size <= MAX_PAGE_BYTES);
        total += size;
    }
    assert!(total <= MAX_RESULT_BYTES);
    Ok(())
}
#[test]
fn oversized_arrays_and_records_refuse_before_family_admission() -> TestResult {
    let (_, pages) = certificate()?;
    let page = pages.first().ok_or("page")?;
    let row = page.content.rows().first().ok_or("row")?.clone();
    let value = json!({"format":1,"subject":hash(1),"compatibility":hash(2),"ordinal":0,"rows":vec![row;16]});
    assert!(serde_json::from_value::<PageContent>(value).is_err());
    assert!(decode_receipt(&hash(1).to_string(), &vec![b' '; MAX_RECEIPT_BYTES]).is_err());
    assert!(
        CaseKey::new(
            RatingClass::I,
            CaseKind::Probe {
                pair: 36,
                name: ProbeName::EmptyArmed
            }
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn jobs_and_completion_bind_exact_immutable_subject_and_publication() -> TestResult {
    let (receipt, pages) = certificate()?;
    let job = RatingJob::new(
        receipt.publication.job,
        receipt.subject().clone(),
        receipt.compatibility().clone(),
        hash(20),
    );
    assert!(
        job.transition(RatingJobState::Completed {
            receipt: receipt.reference()?
        })
        .is_err()
    );
    let running = job.transition(RatingJobState::Running { lease_sequence: 1 })?;
    let completed = running.transition(RatingJobState::Completed {
        receipt: receipt.reference()?,
    })?;
    assert!(completed.matches_receipt(&receipt)?);
    let mut different_job = completed.clone();
    different_job.job = CommitToken::from_bytes([2; 16]);
    assert!(
        !different_job.matches_receipt(&receipt)?,
        "a second completed job cannot reuse the first job's publication"
    );
    let mut different_plan = completed.clone();
    different_plan.plan = hash(21);
    assert!(
        !different_plan.matches_receipt(&receipt)?,
        "identical source is insufficient when the admitted case plan differs"
    );
    assert!(
        running
            .transition(RatingJobState::Running { lease_sequence: 3 })
            .is_err()
    );
    let completed = running.transition(RatingJobState::Completed {
        receipt: receipt.reference()?,
    })?;
    assert!(completed.transition(RatingJobState::Cancelled {}).is_err());
    let execution = test_support::completed_execution(&receipt, &pages)?;
    let completion = RatingCompletion::for_execution(&receipt, &execution)?;
    assert!(completion.matches(&receipt, &execution)?);
    let mut other = receipt.clone();
    other.publication.job = CommitToken::from_bytes([2; 16]);
    assert!(completion.matches(&other, &execution).is_err());
    assert!(
        RatingPublication::new(
            job.job(),
            ClockStamp {
                clock: ClockRef::Utc,
                at: 100
            }
        )
        .is_err()
    );
    Ok(())
}

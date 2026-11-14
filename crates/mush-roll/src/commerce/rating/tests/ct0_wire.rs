//! Doc 46 CT.0: codec pins of the Registry evidence records (`record!`), the two
//! digest ids (`digest_id!`) and every digest over them
//! (`tests/ct0/wire_rating.snap`). The fixture is `test_support::complete_rating`
//! (one probe case) plus a trial case that carries a `TrialEnd`.
use super::*;
use mush_testkit::snapshot::Snapshot;

type Ct0Result = Result<(), Box<dyn std::error::Error>>;

#[test]
fn ct0_rating_wire() -> Ct0Result {
    let mut s = Snapshot::new();
    s.section("constants");
    for (name, value) in [
        ("FORMAT", usize::from(FORMAT)),
        ("MAX_CASES", MAX_CASES),
        ("MAX_PAGE_ROWS", MAX_PAGE_ROWS),
        ("MAX_PAGES", MAX_PAGES),
        ("MAX_CASE_BYTES", MAX_CASE_BYTES),
        ("MAX_PAGE_BYTES", MAX_PAGE_BYTES),
        ("MAX_RECEIPT_BYTES", MAX_RECEIPT_BYTES),
        ("MAX_RESULT_BYTES", MAX_RESULT_BYTES),
        ("TYPED_ENVELOPE_RESERVE", TYPED_ENVELOPE_RESERVE),
    ] {
        s.line(name, &value.to_string());
    }

    let subject = subject()?;
    let compatibility = RatingCompatibility::combat_v1(hash(10));
    let job = CommitToken::from_bytes([1; 16]);
    let complete = test_support::complete_rating(
        subject.clone(),
        compatibility.clone(),
        hash(20),
        job,
        ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 100,
        },
    )?;
    let receipt = &complete.receipt;
    let page = complete.pages.first().ok_or("page")?;
    let content = &receipt.content;
    let probe = page.content.rows.0.first().ok_or("row")?;
    let trial = row(
        RatingClass::I,
        CaseKind::Trial {
            variant: 0,
            layout: TrialLayout::Duel,
            seed: 1,
            swapped: false,
        },
        100,
        20,
    )?;
    let end = trial.terminal.clone().ok_or("trial end")?;
    let alias = content.aliases.0.first().ok_or("alias")?;
    let class = content.classes.0.first().ok_or("class")?;
    let page_ref = content.result.pages.0.first().ok_or("page ref")?;
    let peak = probe.meter.peak.clone().ok_or("peak")?;

    s.section("digests and ids");
    s.wire("subject digest", &subject.digest()?)?;
    s.wire("page content digest", &page.content.digest()?)?;
    s.wire("page ref digest", &page_ref.digest())?;
    s.wire("result id", &content.result.id)?;
    s.wire("class cases digest", &class.cases_digest)?;
    s.wire("certificate id", &content.id()?)?;
    s.wire("receipt id", &receipt.id())?;
    s.wire("receipt reference", &receipt.reference()?)?;
    s.line("page key", &page.key().encoded());
    s.line("receipt id text", &receipt.id().to_string());
    check_eq!(RatingId::from_hex(&receipt.id().to_string())?, receipt.id());
    check_eq!(
        ResultId::from_hex(&content.result.id.to_string())?,
        content.result.id
    );

    // The two digest ids are transparent strings, not objects: their pin is the
    // text form, which must decode back exactly.
    s.section("RatingId / ResultId (digest_id!)");
    s.wire("RatingId", &receipt.id())?;
    s.wire("ResultId", &content.result.id)?;
    check_eq!(
        serde_json::from_value::<RatingId>(serde_json::to_value(receipt.id())?)?,
        receipt.id()
    );
    check_eq!(
        serde_json::from_value::<ResultId>(serde_json::to_value(content.result.id)?)?,
        content.result.id
    );
    s.codec_shape("RatingSubject", &subject)?;
    s.codec_shape("RatingCompatibility", &compatibility)?;
    s.codec("CaseKey (probe)", &probe.key, |j| {
        j["case"]["pair"] = 36.into()
    })?;
    s.codec("CaseKey (trial)", &trial.key, |j| {
        j["case"]["seed"] = 3.into()
    })?;
    s.codec("Peak", &peak, |j| j["unit"] = 33.into())?;
    s.codec("MeterSummary", &probe.meter, |j| {
        j["ops_faults"] = 99.into()
    })?;
    // Doc 46 unit CT.0b (CT.1's finding): a meter that never thought says `"peak":null`, and
    // a document that leaves the key out is refused, not read as a meter with no peak.
    mush_testkit::snapshot::refuses_left_out(
        "MeterSummary (no think, no peak)",
        &MeterSummary::new(0, 0, 0, None, 0, 0, 0, 0)?,
        "peak",
    )?;
    s.codec_shape("CaseBinding", &probe.binding)?;
    s.codec("Coverage", &probe.coverage, |j| {
        j["required_thinks"] = 0.into()
    })?;
    s.codec("TrialEnd", &end, |j| j["at_ms"] = 0.into())?;
    s.codec("CaseResult (probe, no terminal)", probe, |j| {
        j["coverage"]["evaluated_thinks"] = 99.into()
    })?;
    s.codec("CaseResult (trial, terminal)", &trial, |j| {
        j["terminal"] = serde_json::Value::Null
    })?;
    s.codec("PageContent", &page.content, |j| j["format"] = 2.into())?;
    s.codec("RatingPageRef", page_ref, |j| j["rows"] = 0.into())?;
    s.codec("RatingResultRef", &content.result, |j| {
        j["cases"] = 2.into()
    })?;
    s.codec("RatingPage", page, |j| j["content"]["format"] = 2.into())?;
    s.codec("VariantAlias", alias, |j| j["variant"] = 12.into())?;
    s.codec("ClassSummary", class, |j| j["cases"] = 0.into())?;
    s.codec("CertificateContent", content, |j| j["format"] = 2.into())?;
    s.codec("RatingPublication", &receipt.publication, |j| {
        j["issued"]["clock"]["generation"] = 0.into()
    })?;
    s.codec("RatingReceipt", receipt, |j| j["format"] = 2.into())?;
    s.codec("RatingRef", &receipt.reference()?, |j| {
        j["schema"] = 2.into()
    })?;
    let queued = RatingJob::new(job, subject, compatibility, hash(20));
    let states = [
        RatingJobState::Queued {},
        RatingJobState::Running { lease_sequence: 1 },
        RatingJobState::Completed {
            receipt: receipt.reference()?,
        },
        RatingJobState::Failed {
            reason: RatingFailure::Execution,
        },
        RatingJobState::Interrupted {},
        RatingJobState::Cancelled {},
    ];
    for state in states {
        let label = serde_json::to_value(&state)?
            .get("kind")
            .and_then(|k| k.as_str())
            .ok_or("kind")?
            .to_owned();
        let row = RatingJob {
            state,
            ..queued.clone()
        };
        s.codec(&format!("RatingJob ({label})"), &row, |j| {
            j["format"] = 2.into()
        })?;
    }
    check_eq!(
        complete.job.state,
        RatingJobState::Completed {
            receipt: receipt.reference()?
        }
    );
    s.codec("RatingCompletion", &complete.completion, |j| {
        j["format"] = 2.into()
    })?;
    s.codec("RatingStatus (active)", &complete.status, |j| {
        j["format"] = 2.into()
    })?;
    let recalled = RatingStatus {
        standing: RatingStanding::Recalled {
            operation: CommitToken::from_bytes([9; 16]),
        },
        ..complete.status.clone()
    };
    s.codec("RatingStatus (recalled)", &recalled, |j| {
        j["format"] = 2.into()
    })?;
    s.check(env!("CARGO_MANIFEST_DIR"), "tests/ct0/wire_rating.snap")?;
    Ok(())
}

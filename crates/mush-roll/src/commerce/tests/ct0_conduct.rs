//! Doc 46 CT.0: codec pins of the Conduct trial records (`strict!`), the rows
//! `row!` keys into their families, the protocol's actions and every digest
//! (`tests/ct0/wire_conduct.snap`). The rows are built through the public
//! constructors and the strict decoders, as no Conduct test rig exists.
use super::ct0_rig::{self as rig, TestResult};
use crate::commerce::conduct::protocol::{self, Transition};
use crate::commerce::conduct::records::{
    ConductAdmission, ConductExecution, ConductJob, ConductMeasurement, ConductResult, ConductRole,
    ConductSubject,
};
use crate::commerce::rating::execution::ArtifactIdentity;
use crate::commerce::rating::{RatingClass, RatingRef, RatingSubject};
use crate::commerce::records::Record;
use crate::commerce::{CharterId, FiledRevisionId, LineId, RevisionIdentity};
use mush_core::{CommitToken, DefinitionRef};
use mush_testkit::snapshot::Snapshot;
use serde_json::json;

use mush_testkit::fixtures::digest as hash;
use mush_testkit::fixtures::token;

fn subject() -> TestResult<ConductSubject> {
    let revision: RevisionIdentity = serde_json::from_value(json!({"format":1,
        "revision":{"key":{"character":7,"mind":1,"number":1},"digest":hash(1)},"source":hash(2)}))?;
    let rating: RatingRef = serde_json::from_value(
        json!({"job":token(30),"id":hash(31),"schema":1,"receipt_digest":hash(32)}),
    )?;
    Ok(ConductSubject::captured(
        CharterId::new(9)?,
        LineId::new(2)?,
        FiledRevisionId::new(3)?,
        revision,
        rating,
        RatingSubject::new(revision, hash(3), hash(9), hash(4), hash(5), RatingClass::I),
        "Line".into(),
        ConductRole::Escort,
    )?)
}

fn job(manifest: &DefinitionRef) -> TestResult<ConductJob> {
    let subject = subject()?;
    let admission = ConductAdmission::observed(
        hash(40),
        mush_core::GOD,
        1,
        1,
        subject.worker_digest()?,
        hash(41),
        hash(42),
        hash(43),
        ArtifactIdentity {
            sha256: hash(44),
            bytes: 100,
        },
        1_000_000,
    )?;
    Ok(ConductJob::observed(
        token(10),
        protocol::action_ref(manifest, Transition::Admit)?,
        manifest.clone(),
        subject,
        admission,
        20,
    )?)
}

fn execution(
    job: &ConductJob,
    manifest: &DefinitionRef,
    transition: Transition,
    updated_by: CommitToken,
    observation: serde_json::Value,
) -> TestResult<ConductExecution> {
    let row: ConductExecution = serde_json::from_value(json!({"format":1,"job":job.operation(),
        "updated_by":updated_by,"action":protocol::action_ref(manifest, transition)?,
        "observation":observation}))?;
    row.matches_job(job)?;
    Ok(row)
}

#[test]
fn ct0_conduct_wire() -> TestResult {
    let (_engine, f) = rig::families()?;
    let mut s = Snapshot::new();
    s.section("families");
    for (name, value) in [
        ("JOB_BYTES", crate::commerce::conduct::JOB_BYTES),
        ("EXECUTION_BYTES", crate::commerce::conduct::EXECUTION_BYTES),
        ("RESULT_BYTES", crate::commerce::conduct::RESULT_BYTES),
        ("MAX_TRIALS", crate::commerce::conduct::MAX_TRIALS),
    ] {
        s.line(name, &value.to_string());
    }
    rig::family(&mut s, &f.conduct_jobs);
    rig::family(&mut s, &f.conduct_executions);
    rig::family(&mut s, &f.conduct_results);

    s.section("protocol");
    let manifest = rig::reference("tacsim", "conduct_manifest", "safe_conduct")?;
    s.wire("manifest", &manifest)?;
    for t in Transition::ALL {
        s.line(&format!("transition {} action id", t.word()), t.action_id());
        let registered = rig::descriptor(
            &mut s,
            &format!("action {}", t.word()),
            "roll",
            &protocol::action_descriptor(&manifest, t)?,
        )?;
        check_eq!(protocol::action_ref(&manifest, t)?, registered);
    }

    s.section("job");
    let job = job(&manifest)?;
    check_eq!(job.key(), job.operation().to_hex());
    rig::document_keyed(&mut s, "job", &f.conduct_jobs, &job)?;
    s.wire("job digest", &job.digest()?)?;
    s.wire("subject worker digest", &job.subject().worker_digest()?)?;

    s.section("executions");
    let queued = execution(
        &job,
        &manifest,
        Transition::Admit,
        job.operation(),
        json!({"phase":"queued","next_sequence":1,"cursor":{"index":0,"simulated_ms":0},
            "charged_unit_ms":0,"flight":null,"result":null}),
    )?;
    let running = execution(
        &job,
        &manifest,
        Transition::Grant,
        token(11),
        json!({"phase":"running","next_sequence":2,"cursor":{"index":3,"simulated_ms":500},
            "charged_unit_ms":10,"flight":null,"result":null}),
    )?;
    let artifact = ArtifactIdentity {
        sha256: hash(45),
        bytes: 200,
    };
    let complete = execution(
        &job,
        &manifest,
        Transition::Settle,
        token(12),
        json!({"phase":"complete","next_sequence":3,"cursor":{"index":59,"simulated_ms":120_000},
            "charged_unit_ms":1000,"flight":null,"result":artifact}),
    )?;
    let cancelled = execution(
        &job,
        &manifest,
        Transition::Cancel,
        token(13),
        json!({"phase":"cancelled","next_sequence":3,"cursor":{"index":3,"simulated_ms":500},
            "charged_unit_ms":10,"flight":null,"result":null}),
    )?;
    for (label, row) in [
        ("queued", &queued),
        ("running", &running),
        ("complete", &complete),
        ("cancelled", &cancelled),
    ] {
        rig::document_keyed(
            &mut s,
            &format!("{label} execution"),
            &f.conduct_executions,
            row,
        )?;
        s.wire(&format!("{label} execution digest"), &row.digest()?)?;
    }

    s.section("results");
    let measurement = ConductMeasurement::observed(
        hash(46),
        job.admission().policy(),
        job.admission().subject(),
        1000,
        true,
    )?;
    let passed: ConductResult = serde_json::from_value(json!({"format":1,"job":job.operation(),
        "operation":complete.updated_by(),"action":protocol::action_ref(&manifest, Transition::Settle)?,
        "job_digest":job.digest()?,"execution_digest":complete.digest()?,"phase":"complete",
        "artifact":artifact,"measurement":measurement}))?;
    passed.matches(&job, &complete)?;
    check!(passed.qualifies());
    let withdrawn: ConductResult = serde_json::from_value(
        json!({"format":1,"job":job.operation(),
        "operation":cancelled.updated_by(),"action":protocol::action_ref(&manifest, Transition::Cancel)?,
        "job_digest":job.digest()?,"execution_digest":cancelled.digest()?,"phase":"cancelled",
        "artifact":null,"measurement":null}),
    )?;
    withdrawn.matches(&job, &cancelled)?;
    // Doc 46 unit CT.0b (CT.1's finding): a cancelled result says `null` for both, and a
    // document that leaves either key out is refused.
    for key in ["artifact", "measurement"] {
        mush_testkit::snapshot::refuses_left_out("ConductResult (cancelled)", &withdrawn, key)?;
    }
    for (label, row) in [("passed", &passed), ("cancelled", &withdrawn)] {
        rig::document_keyed(&mut s, &format!("{label} result"), &f.conduct_results, row)?;
        s.wire(&format!("{label} result digest"), &row.digest()?)?;
    }

    s.codec("ConductSubject", job.subject(), |j| {
        j["profile"] = " Line".into()
    })?;
    s.codec("ConductAdmission", job.admission(), |j| j["run"] = 0.into())?;
    s.codec("ConductJob", &job, |j| j["format"] = 2.into())?;
    for (label, row) in [
        ("queued", &queued),
        ("running", &running),
        ("complete", &complete),
        ("cancelled", &cancelled),
    ] {
        s.codec(&format!("ConductExecution ({label})"), row, |j| {
            j["format"] = 2.into()
        })?;
    }
    s.codec("ConductMeasurement", &measurement, |j| {
        j["paired_cases"] = 29.into()
    })?;
    s.codec("ConductResult (complete)", &passed, |j| {
        j["measurement"] = serde_json::Value::Null
    })?;
    s.codec("ConductResult (cancelled)", &withdrawn, |j| {
        j["format"] = 2.into()
    })?;
    s.check(env!("CARGO_MANIFEST_DIR"), "tests/ct0/wire_conduct.snap")?;
    Ok(())
}

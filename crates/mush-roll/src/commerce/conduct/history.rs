//! Full typed/native/source inverse. Live metadata may omit certificate pages
//! only after the actual complete Roll boot audit has opened commerce.
use super::{protocol::ConductHistoryRow, records::*, state::Configuration, *};
use crate::{
    RollStore,
    commerce::{
        families::CommerceFamilies,
        read::CommerceRead,
        records::{Record, canonical_duplicate_digest, schema_line_digest},
        roots::{RequiredRoot, RootAnchor},
    },
};
use mush_core::{CommitToken, ModuleCtx, StateFamilyHandle};
use std::collections::BTreeSet;

pub(super) fn row<'a, R: Record>(
    ctx: &'a impl CommerceRead,
    family: &StateFamilyHandle<String, R>,
    key: String,
) -> Result<&'a R, PublicationError> {
    let row = ctx
        .get(family, &key)
        .map_err(|_| PublicationError::Unavailable)?
        .ok_or(PublicationError::Missing)?;
    row.validate()?;
    if row.key() != key {
        return Err(PublicationError::Mismatch);
    }
    Ok(row)
}
pub(crate) fn job_root(job: &ConductJob) -> Result<RequiredRoot, PublicationError> {
    Ok(RequiredRoot {
        anchor: RootAnchor::ConductJob(job.operation(), job.digest()?),
        identity: job.subject().revision(),
    })
}
pub(super) fn subject(
    ctx: &impl CommerceRead,
    roll: &RollStore,
    f: &CommerceFamilies,
    config: &Configuration,
    job: &ConductJob,
    complete_certificate: bool,
) -> Result<(), PublicationError> {
    job.validate()?;
    let s = job.subject();
    let filed = row(ctx, &f.filed, s.filed().key())?;
    let line = row(ctx, &f.lines, s.line().key())?;
    let receipt = config
        .ratings
        .resolve_receipt_read(ctx, &s.rating())
        .map_err(invalid)?;
    if filed.content.revision() != s.revision()
        || filed.content.position().line != s.line()
        || filed.content.position().charter != s.charter()
        || line.charter != s.charter()
        || receipt.subject() != s.worker()
        || receipt.subject().definition() != filed.definition
        || receipt.subject().schema() != line.schema_digest
    {
        return Err(PublicationError::Mismatch);
    }
    if complete_certificate {
        config
            .ratings
            .complete_receipt(ctx, receipt)
            .map_err(invalid)?;
    }
    let reference = s.revision().revision();
    let revision = roll
        .revision(reference.key.character, reference.key)
        .map_err(invalid)?;
    if !s.revision().matches(revision).map_err(invalid)?
        || schema_line_digest(revision.envelope())? != line.schema_digest
        || canonical_duplicate_digest(revision)? != filed.duplicate
    {
        return Err(PublicationError::Mismatch);
    }
    for root in [
        RequiredRoot {
            anchor: RootAnchor::Filed(s.filed()),
            identity: s.revision(),
        },
        RequiredRoot {
            anchor: RootAnchor::Rating {
                job: receipt.publication().job(),
                id: receipt.id(),
            },
            identity: s.revision(),
        },
        job_root(job)?,
    ] {
        roll.check_required_root(&root).map_err(invalid)?;
    }
    Ok(())
}

pub(crate) fn audit(
    ctx: &ModuleCtx<'_>,
    roll: &RollStore,
    f: &CommerceFamilies,
    enabled: bool,
) -> Result<(), PublicationError> {
    let jobs = collect(ctx, &f.conduct_jobs)?;
    let executions = collect(ctx, &f.conduct_executions)?;
    let results = collect(ctx, &f.conduct_results)?;
    if jobs.len() > MAX_TRIALS || executions.len() != jobs.len() || results.len() > jobs.len() {
        return Err(PublicationError::Capacity);
    }
    let anchors = roll
        .commercial_roots()
        .collect::<Result<Vec<_>, _>>()
        .map_err(invalid)?;
    if !enabled {
        if !jobs.is_empty()
            || !executions.is_empty()
            || !results.is_empty()
            || anchors
                .iter()
                .any(|(a, _)| matches!(a, RootAnchor::ConductJob(..)))
        {
            return Err(PublicationError::Unavailable);
        }
        return Ok(());
    }
    let config = roll.conduct.configuration()?;
    let mut expected = BTreeSet::new();
    let mut views = Vec::with_capacity(jobs.len());
    let mut terminal = 0;
    for job in &jobs {
        let execution = row(ctx, &f.conduct_executions, job.operation().to_hex())?;
        execution.matches_job(job)?;
        let result = ctx
            .get(&f.conduct_results, &job.operation().to_hex())
            .map_err(|_| PublicationError::Unavailable)?;
        subject(ctx, roll, f, config, job, true)?;
        expected.insert(job_root(job)?.anchor);
        match result {
            Some(result) => {
                result.matches(job, execution)?;
                terminal += 1;
            }
            None if execution.observation().phase.terminal() => {
                return Err(PublicationError::Missing);
            }
            None => {}
        }
        views.push(ConductHistoryRow {
            job,
            execution,
            result,
        });
    }
    if terminal != results.len() {
        return Err(PublicationError::Mismatch);
    }
    let actual = anchors
        .into_iter()
        .filter_map(|(a, _)| matches!(&a, RootAnchor::ConductJob(..)).then_some(a))
        .collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(PublicationError::Mismatch);
    }
    // Mandatory even for empty typed inventory; missing all typed halves is not empty native history.
    config.native.audit(ctx, &views)
}

pub(crate) fn metadata(
    ctx: &ModuleCtx<'_>,
    f: &CommerceFamilies,
    operation: CommitToken,
) -> Result<super::ConductTrialProof, PublicationError> {
    let roll = ctx
        .state::<RollStore>()
        .ok_or(PublicationError::Unavailable)?;
    roll.coverage_audit_ready().map_err(invalid)?;
    super::reader::proof(ctx, roll, f, operation, true)
}

fn collect<R: Record>(
    ctx: &ModuleCtx<'_>,
    family: &StateFamilyHandle<String, R>,
) -> Result<Vec<R>, PublicationError> {
    let mut rows = Vec::new();
    super::super::audit::scan(ctx, family, |row| {
        if rows.len() == MAX_TRIALS {
            return Err(PublicationError::Capacity);
        }
        rows.push(row.clone());
        Ok(())
    })?;
    Ok(rows)
}

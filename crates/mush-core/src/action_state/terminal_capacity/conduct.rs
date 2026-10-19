//! Closed completion capacity for actual paid Conduct worker trials. This
//! proves storage and typed affinity; native owners prove execution and source.
use super::*;
use crate::action::PrincipalId;
mod protocol;
#[cfg(test)]
mod tests;
mod transition;
mod wire;
pub(in crate::action_state) use protocol::{Protocol, RequiredImages};
pub(in crate::action_state) use transition::conduct_change;
pub(in crate::action_state::terminal_capacity) use transition::{audit_boot, validate_changes};

const JOB: &str = "roll_conduct_trial_job";
const EXECUTION: &str = "roll_conduct_trial_execution";
const RESULT: &str = "roll_conduct_trial_result";
const FAMILIES: [(&str, usize); 3] = [(JOB, 8192), (EXECUTION, 4096), (RESULT, 4096)];
const MAX_CHARGE: u64 = 60 * 4 * 121_000;
fn bad(_: impl std::fmt::Display) -> ActionStateError {
    refused("Conduct encoding or affinity")
}
fn owner(s: &str) -> Result<OwnerId, ActionStateError> {
    OwnerId::new(s).map_err(bad)
}
fn parsed<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, ActionStateError> {
    serde_json::from_value(v.clone()).map_err(bad)
}
fn field<'a>(v: &'a Value, n: &str) -> Result<&'a Value, ActionStateError> {
    v.get(n).ok_or_else(|| refused("Conduct required field"))
}
fn shape(v: &Value, names: &[&str]) -> Result<(), ActionStateError> {
    let o = v.as_object().ok_or_else(|| refused("Conduct object"))?;
    if o.len() != names.len() || names.iter().any(|n| !o.contains_key(*n)) {
        return Err(refused("Conduct closed shape"));
    }
    Ok(())
}
fn number(v: &Value, n: &str) -> Result<u64, ActionStateError> {
    field(v, n)?
        .as_u64()
        .ok_or_else(|| refused("Conduct unsigned integer"))
}
fn digest(domain: &str, v: &Value, bytes: usize) -> Result<DefinitionDigest, ActionStateError> {
    DefinitionDigest::for_canonical_value_with_limit(domain, v, bytes).map_err(bad)
}
fn key(family: &str, job: CommitToken) -> Result<Key, ActionStateError> {
    Ok(Key {
        owner: owner("roll")?,
        family: StateFamilyId::new(family).map_err(bad)?,
        encoded: job.to_hex().into(),
    })
}
fn document(view: &View<'_>, k: &Key) -> Result<Option<Value>, ActionStateError> {
    let k = k.resolve(view.registry)?;
    match view.mutations.and_then(|m| m.get(&k)) {
        Some(TxnMutation::Put { document, .. }) => Ok(Some(document.clone())),
        Some(TxnMutation::Delete) => Ok(None),
        None => view
            .records
            .get(&k)
            .map(|r| serde_json::from_slice(r.canonical.as_slice()).map_err(bad))
            .transpose(),
    }
}
fn get(view: &View<'_>, family: &str, job: CommitToken) -> Result<Option<Value>, ActionStateError> {
    document(view, &key(family, job)?)?
        .map(|v| field(&v, "value").cloned())
        .transpose()
}
fn required(view: &View<'_>, family: &str, job: CommitToken) -> Result<Value, ActionStateError> {
    get(view, family, job)?.ok_or_else(|| refused("Conduct missing counterpart"))
}
fn ready(view: &View<'_>) -> Result<(), ActionStateError> {
    for o in ["roll", "tacsim"] {
        view.registry
            .owner_readiness
            .require_ready(&owner(o)?)
            .map_err(ActionStateError::OwnerNotReady)?;
    }
    Ok(())
}
fn targets(job: CommitToken) -> Result<Vec<Target>, ActionStateError> {
    [
        (EXECUTION, SlotMode::UpdateExisting),
        (RESULT, SlotMode::Create),
    ]
    .into_iter()
    .map(|(f, mode)| {
        Ok(Target {
            key: key(f, job)?,
            schema: protocol::descriptor(f, 4096)?.value_schema().clone(),
            schema_version: 1,
            ceiling: 4096,
            mode,
        })
    })
    .collect()
}
fn pair(
    view: &View<'_>,
    job_id: CommitToken,
) -> Result<(wire::Job, wire::Execution), ActionStateError> {
    let job = wire::Job::parse(required(view, JOB, job_id)?, job_id)?;
    let execution = wire::Execution::parse(required(view, EXECUTION, job_id)?, &job)?;
    match get(view, RESULT, job_id)? {
        Some(result) if execution.terminal() => wire::result(&result, &job, &execution)?,
        None if !execution.terminal() => {}
        _ => return Err(refused("Conduct exact terminal result presence")),
    }
    Ok((job, execution))
}
pub(super) fn verify(
    phase: Phase,
    reservation: &Reservation,
    view: &View<'_>,
) -> Result<(), ActionStateError> {
    ready(view)?;
    let id = CommitToken::from_hex(&reservation.sponsor.encoded).map_err(bad)?;
    let (job, execution) = pair(view, id)?;
    if reservation.sponsor != key(JOB, id)?
        || reservation.funding != id
        || reservation.effect != job.action
        || reservation.targets != targets(id)?
        || reservation.protocol_fingerprint != Some(job.digest()?)
    {
        return Err(refused("Conduct exact reserved relation"));
    }
    match phase {
        Phase::Funding if execution.phase != "queued" => {
            return Err(refused("Conduct admission is queued"));
        }
        Phase::Retained if execution.terminal() => {
            return Err(refused("Conduct live reservation on terminal"));
        }
        Phase::Terminal if !execution.terminal() => {
            return Err(refused("Conduct terminal proof required"));
        }
        _ => {}
    }
    wire::future_room(view, &job)?;
    Ok(())
}

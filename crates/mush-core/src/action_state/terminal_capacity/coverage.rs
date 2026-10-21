//! Fixed hull coverage completion. Native Economy and Roll adapters retain
//! monetary, source and host authority; this owner reserves six exact keys and
//! checks their complete typed relation before any union can take custody.
use super::*;
use crate::action::PrincipalId;
use crate::{ClockStamp, ObjId};
mod history;
mod protocol;
#[cfg(test)]
mod tests;
mod transition;
mod wire;
pub(in crate::action_state) use protocol::{Protocol, RequiredImages};
pub(in crate::action_state) use transition::coverage_change;
use wire::Row;

const POLICY: &str = "tacsim_coverage_policies";
const EXPOSURE: &str = "tacsim_coverage_exposures";
const HULL: &str = "tacsim_coverage_hulls";
const RETENTION: &str = "roll_mind_coverage";
const TERMINAL: &str = "tacsim_coverage_terminals";
const RECEIPT: &str = "tacsim_coverage_receipts";
const COUNTS: &str = "tacsim_coverage_counts";
const QUOTE: &str = "tacsim_coverage_quotes";
const PRODUCT: &str = "tacsim_coverage_products";
const FAMILIES: [(&str, &str, usize); 6] = [
    ("tacsim", POLICY, 81920),
    ("tacsim", EXPOSURE, 4096),
    ("tacsim", HULL, 4096),
    ("roll", RETENTION, 16384),
    ("tacsim", TERMINAL, 16384),
    ("tacsim", RECEIPT, 8192),
];
const PREREQUISITES: [(&str, &str, usize); 3] = [
    ("tacsim", COUNTS, 1024),
    ("tacsim", QUOTE, 16384),
    ("tacsim", PRODUCT, 98304),
];
fn bad(_: impl std::fmt::Display) -> ActionStateError {
    refused("coverage encoding or affinity")
}
fn owner(s: &str) -> Result<OwnerId, ActionStateError> {
    OwnerId::new(s).map_err(bad)
}
fn field<'a>(v: &'a Value, n: &str) -> Result<&'a Value, ActionStateError> {
    v.get(n).ok_or_else(|| refused("coverage required field"))
}
fn parsed<T: serde::de::DeserializeOwned>(v: &Value) -> Result<T, ActionStateError> {
    serde_json::from_value(v.clone()).map_err(bad)
}
fn shape(v: &Value, names: &[&str]) -> Result<(), ActionStateError> {
    let o = v.as_object().ok_or_else(|| refused("coverage object"))?;
    if o.len() != names.len() || names.iter().any(|n| !o.contains_key(*n)) {
        if std::env::var_os("M7_CONDUCT_DIAG").is_some() {
            // TEMP M7 diag: key names only.
            eprintln!("M7_CONDUCT_DIAG coverage shape expected={names:?} actual={:?}", o.keys().collect::<Vec<_>>());
        }
        return Err(refused("coverage closed shape"));
    }
    Ok(())
}
fn number(v: &Value, n: &str) -> Result<u64, ActionStateError> {
    field(v, n)?
        .as_u64()
        .ok_or_else(|| refused("coverage unsigned integer"))
}
fn digest(domain: &str, v: &Value, bytes: usize) -> Result<DefinitionDigest, ActionStateError> {
    DefinitionDigest::for_canonical_value_with_limit(domain, v, bytes).map_err(bad)
}
fn key(family: &str, encoded: String) -> Result<Key, ActionStateError> {
    let (o, _, _) = FAMILIES
        .iter()
        .chain(&PREREQUISITES)
        .find(|(_, f, _)| *f == family)
        .ok_or_else(|| refused("coverage unknown target"))?;
    Ok(Key {
        owner: owner(o)?,
        family: StateFamilyId::new(family).map_err(bad)?,
        encoded: encoded.into(),
    })
}
fn get(view: &View<'_>, raw: &Key) -> Result<Option<Value>, ActionStateError> {
    let k = raw.resolve(view.registry)?;
    let document = match view.mutations.and_then(|m| m.get(&k)) {
        Some(TxnMutation::Put { document, .. }) => Some(document.clone()),
        Some(TxnMutation::Delete) => None,
        None => view
            .records
            .get(&k)
            .map(|r| serde_json::from_slice::<Value>(r.canonical.as_slice()).map_err(bad))
            .transpose()?,
    };
    document.map(|d| field(&d, "value").cloned()).transpose()
}
fn required(view: &View<'_>, k: &Key) -> Result<Value, ActionStateError> {
    get(view, k)?.ok_or_else(|| refused("coverage missing counterpart"))
}
fn ready(view: &View<'_>) -> Result<(), ActionStateError> {
    for o in ["tacsim", "economy", "roll"] {
        view.registry
            .owner_readiness
            .require_ready(&owner(o)?)
            .map_err(ActionStateError::OwnerNotReady)?;
    }
    Ok(())
}
fn row(view: &View<'_>, binding: CommitToken) -> Result<Row, ActionStateError> {
    Row::parse(required(view, &key(POLICY, binding.to_hex())?)?, binding)
}
fn targets(r: &Row) -> Result<Vec<Target>, ActionStateError> {
    let mut result = Vec::new();
    for (o, f, ceiling) in FAMILIES {
        let descriptor = protocol::descriptor(o, f, ceiling)?;
        result.push(Target {
            key: r.key(f)?,
            schema: descriptor.value_schema().clone(),
            schema_version: 1,
            ceiling,
            mode: if [TERMINAL, RECEIPT].contains(&f) {
                SlotMode::Create
            } else {
                SlotMode::UpdateExisting
            },
        });
    }
    result.sort_by(|a, b| {
        (&a.key.owner, &a.key.family, &a.key.encoded).cmp(&(
            &b.key.owner,
            &b.key.family,
            &b.key.encoded,
        ))
    });
    Ok(result)
}

pub(super) fn verify(
    phase: Phase,
    reservation: &Reservation,
    view: &View<'_>,
) -> Result<(), ActionStateError> {
    ready(view)?;
    let binding = CommitToken::from_hex(&reservation.sponsor.encoded).map_err(bad)?;
    let r = row(view, binding)?;
    if reservation.sponsor != r.key(POLICY)?
        || reservation.funding != binding
        || reservation.effect != r.action
        || reservation.protocol_fingerprint != Some(r.fingerprint(view)?)
        || reservation.targets != targets(&r)?
    {
        return Err(refused("coverage exact reserved relation"));
    }
    match phase {
        Phase::Funding | Phase::Retained if r.terminal()?.is_some() => {
            return Err(refused("coverage live reservation on terminal"));
        }
        Phase::Terminal if r.terminal()?.is_none() => {
            return Err(refused("coverage incomplete terminal"));
        }
        _ => {}
    }
    r.paired(view)?;
    r.future_room(view)?;
    Ok(())
}
pub(in crate::action_state::terminal_capacity) use transition::{audit_boot, validate_changes};

//! Future complete-document size accounting only. Null slots below stand for
//! separately enforced native bounds; they are never persisted or decoded as
//! receipts, authorizations, host evidence or participant outputs.
use super::{
    CoverageError,
    families::{Families, Row},
    policy::{Exposure, HullHead, Policy, Receipt, Terminal},
};
use mush_core::{StateFamilyHandle, canonical_json_bytes_with_limit};
use mush_economy::managed_coverage::MAX_HISTORY_BYTES;
fn bad(_: impl std::fmt::Display) -> CoverageError {
    CoverageError::Bound
}
fn complete<R: Row>(
    family: &StateFamilyHandle<String, R>,
    key: String,
    value: serde_json::Value,
    slots: &[usize],
) -> Result<(), CoverageError> {
    let d = family.descriptor();
    if family.owner().as_str() != crate::NS
        || d.id().as_str() != R::FAMILY
        || d.schema_version() != 1
        || d.max_record_bytes() != R::LIMIT
    {
        return Err(CoverageError::Product);
    }
    let doc = serde_json::json!({"family":{"owner":family.owner(),"id":d.id()},"key":key,
        "schema":{"id":d.value_schema(),"version":d.schema_version()},"value":value});
    let mut bytes = canonical_json_bytes_with_limit(&doc, R::LIMIT)
        .map_err(bad)?
        .len();
    for bound in slots {
        bytes = bytes
            .checked_sub(4)
            .and_then(|n| n.checked_add(*bound))
            .ok_or(CoverageError::Bound)?;
    }
    if bytes > R::LIMIT {
        return Err(CoverageError::Bound);
    }
    Ok(())
}
pub(super) fn before_money(f: &Families, p: &Policy) -> Result<(), CoverageError> {
    p.validate()?;
    if p.history.terminal().is_some() {
        return Err(CoverageError::Product);
    }
    let mut policy = serde_json::to_value(p).map_err(bad)?;
    policy["history"] = serde_json::Value::Null;
    complete(&f.policies, p.key(), policy, &[MAX_HISTORY_BYTES])?;
    let marker = p.operation; // All future tokens/digests have fixed canonical width.
    let mut exposure = serde_json::to_value(Exposure::from_policy(p)?).map_err(bad)?;
    exposure["state"] = serde_json::json!({"kind":"released","terminal":marker});
    complete(&f.exposures, p.key(), exposure, &[])?;
    let hull = HullHead::from_policy(p)?;
    complete(
        &f.hulls,
        hull.key(),
        serde_json::to_value(&hull).map_err(bad)?,
        &[],
    )?;
    let terminal = serde_json::json!({"format":1,"request":p.request,"operation":marker,
        "action":p.protocol.action(4)?,"protocol":p.protocol,"policy":p.operation,
        "policy_digest":p.digest()?,"native_history":p.history.digest().map_err(bad)?,"close":null});
    complete::<Terminal>(&f.terminals, p.key(), terminal, &[4096])?;
    let receipt = serde_json::json!({"format":1,"operation":marker,"policy":p.operation,
        "history":p.history.digest().map_err(bad)?,"transaction":mush_economy::TransactionId(u64::MAX),
        "journal_seq":u64::MAX,"state":{"kind":"released","terminal":marker}});
    complete::<Receipt>(&f.receipts, p.key(), receipt, &[])
}

//! Bound the complete future typed documents at Draft, including the largest
//! admitted opaque receipt encodings. Placeholder nulls are size arithmetic;
//! they are never decoded, inserted, or treated as economic evidence.
use super::Issuer;
use super::families::Families;
use super::records::{OperationRow, Row};
use mush_core::{CommitToken, StateFamilyHandle, canonical_json_bytes_with_limit};
use mush_roll::commerce::renewal_twin::Binding as _;
use serde::Serialize;
use serde_json::{Value, json};
fn bad(_: impl std::fmt::Display) -> String {
    super::records::invalid()
}
fn document<R: Serialize + 'static>(
    f: &StateFamilyHandle<CommitToken, R>,
    operation: CommitToken,
    row: &R,
) -> Result<Value, String> {
    let d = f.descriptor();
    if f.owner().as_str() != "market" || d.schema_version() != 1 {
        return Err(super::records::invalid());
    }
    Ok(
        json!({"family":{"owner":f.owner(),"id":d.id()},"key":operation.to_hex(),
        "schema":{"id":d.value_schema(),"version":d.schema_version()},"value":row}),
    )
}
fn room(doc: &mut Value, state: Value, extra: usize, max: usize) -> Result<(), String> {
    *doc.get_mut("value")
        .and_then(|v| v.get_mut("state"))
        .ok_or_else(super::records::invalid)? = state;
    let n = canonical_json_bytes_with_limit(doc, max)
        .map_err(bad)?
        .len();
    if n.checked_add(extra).is_none_or(|n| n > max) {
        return Err(super::records::invalid());
    }
    Ok(())
}
pub(crate) fn check<I: Issuer>(f: &Families<I>, row: &I::Renewal) -> Result<(), String> {
    row.validate()?;
    let cap = mush_economy::renewal_twin::MAX_RENEWAL_RECEIPT_BYTES;
    let one = cap.checked_sub(4).ok_or_else(super::records::invalid)?;
    let two = one.checked_mul(2).ok_or_else(super::records::invalid)?;
    if f.operations.descriptor().max_record_bytes() != I::Renewal::MAX_BYTES
        || f.receipts.descriptor().max_record_bytes() != I::Receipt::MAX_BYTES
    {
        return Err(super::records::invalid());
    }
    let mut operation = document(&f.operations, row.operation(), row)?;
    room(
        &mut operation,
        json!({"kind":"refunded","funding":null,"terminal":null}),
        two,
        I::Renewal::MAX_BYTES,
    )?;
    room(
        &mut operation,
        json!({"kind":"unfunded_closed","closure":null}),
        one,
        I::Renewal::MAX_BYTES,
    )?;
    let d = f.receipts.descriptor();
    let mut receipt = json!({"family":{"owner":f.receipts.owner(),"id":d.id()},"key":row.key(),
        "schema":{"id":d.value_schema(),"version":d.schema_version()},"value":{
            "format":1,"operation":row.operation(),"licence":row.binding().licence(),"predecessor":row.binding().predecessor(),
            "next":row.binding().next(),"contract":row.contract(),"consent_hash":row.consent_hash(),"state":null}});
    room(
        &mut receipt,
        json!({"kind":"refunded","funding":null,"terminal":null}),
        two,
        I::Receipt::MAX_BYTES,
    )?;
    room(
        &mut receipt,
        json!({"kind":"unfunded_closed","closure":null}),
        one,
        I::Receipt::MAX_BYTES,
    )?;
    Ok(())
}

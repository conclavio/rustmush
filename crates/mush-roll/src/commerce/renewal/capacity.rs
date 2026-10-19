//! Admission only: reserve the actual complete Roll operation envelope for
//! both bounded receipts. Placeholders below are size arithmetic, never rows
//! or proof. They are not inserted, decoded as authority, or returned to callers.
use super::super::{
    families::CommerceFamilies,
    records::{PublicationError, Record},
};
use super::{RenewalHeadV1, RenewalOperationV1};
use mush_core::{StateFamilyHandle, canonical_json_bytes_with_limit};
use serde::Serialize;
fn document<T: Record + Serialize>(
    family: &StateFamilyHandle<String, T>,
    row: &T,
) -> Result<serde_json::Value, PublicationError> {
    let d = family.descriptor();
    if family.owner().as_str() != crate::NS
        || d.id().as_str() != T::FAMILY
        || d.schema_version() != 1
        || d.max_record_bytes() != T::MAX_BYTES
    {
        return Err(PublicationError::Mismatch);
    }
    Ok(
        serde_json::json!({"family":{"owner":family.owner(),"id":d.id()},"key":row.key(),
        "schema":{"id":d.value_schema(),"version":d.schema_version()},"value":row}),
    )
}
pub(crate) fn check(
    f: &CommerceFamilies,
    row: &RenewalOperationV1,
) -> Result<(), PublicationError> {
    row.validate()?;
    let mut doc = document(&f.renewal_operations, row)?;
    let state = doc
        .get_mut("value")
        .and_then(|v| v.get_mut("state"))
        .ok_or(PublicationError::Invalid)?;
    // Both E4 public receipt codecs enforce this exact bound. Null occupies
    // four bytes; replacing each null by any valid receipt adds at most cap-4.
    *state = serde_json::json!({"kind":"refunded","funding":null,"terminal":null});
    let cap = mush_economy::managed_renewal::MAX_RENEWAL_RECEIPT_BYTES;
    let two = cap
        .checked_sub(4)
        .and_then(|n| n.checked_mul(2))
        .ok_or(PublicationError::Capacity)?;
    let n = canonical_json_bytes_with_limit(&doc, RenewalOperationV1::MAX_BYTES)
        .map_err(|_| PublicationError::Capacity)?
        .len();
    if n.checked_add(two)
        .is_none_or(|n| n > RenewalOperationV1::MAX_BYTES)
    {
        return Err(PublicationError::Capacity);
    }
    let state = doc
        .get_mut("value")
        .and_then(|v| v.get_mut("state"))
        .ok_or(PublicationError::Invalid)?;
    *state = serde_json::json!({"kind":"unfunded_closed","closure":null});
    let n = canonical_json_bytes_with_limit(&doc, RenewalOperationV1::MAX_BYTES)
        .map_err(|_| PublicationError::Capacity)?
        .len();
    if n.checked_add(cap.checked_sub(4).ok_or(PublicationError::Capacity)?)
        .is_none_or(|n| n > RenewalOperationV1::MAX_BYTES)
    {
        return Err(PublicationError::Capacity);
    }
    let head = RenewalHeadV1::for_operation(row)?;
    let mut head_doc = document(&f.renewal_heads, &head)?;
    let value = head_doc
        .get_mut("value")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or(PublicationError::Invalid)?;
    value.insert("current".into(), serde_json::json!(u64::MAX));
    value.insert("attempted_predecessor".into(), serde_json::json!(u64::MAX));
    // Keep a full token as the largest pending shape, including terminal rows.
    value.insert(
        "pending".into(),
        serde_json::to_value(row.operation()).map_err(|_| PublicationError::Invalid)?,
    );
    canonical_json_bytes_with_limit(&head_doc, RenewalHeadV1::MAX_BYTES)
        .map_err(|_| PublicationError::Capacity)?;
    Ok(())
}

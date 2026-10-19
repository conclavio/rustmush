//! Admission-only headroom. The future term key is fixed-width hex, while
//! the serialized generation is canonical decimal and can grow to 20 digits.
//! Use the actual registered envelope, never a guessed constant subtraction.
use super::super::{
    TermGeneration,
    records::{PublicationError, Record},
};
use super::HouseTermEconomicsV1;
use mush_core::{StateFamilyHandle, canonical_json_bytes_with_limit};

pub(crate) fn check(
    family: &StateFamilyHandle<String, HouseTermEconomicsV1>,
    row: &HouseTermEconomicsV1,
) -> Result<(), PublicationError> {
    let maximum = HouseTermEconomicsV1::capture(
        row.licence(),
        TermGeneration::new(u64::MAX).map_err(|_| PublicationError::Capacity)?,
        row.operation(),
        row.product_digest(),
        row.grant().clone(),
    )?;
    let descriptor = family.descriptor();
    if family.owner().as_str() != "roll"
        || descriptor.id().as_str() != HouseTermEconomicsV1::FAMILY
        || descriptor.schema_version() != 1
        || descriptor.max_record_bytes() != 8192
    {
        return Err(PublicationError::Mismatch);
    }
    let document = serde_json::json!({
        "family":{"owner":family.owner(),"id":descriptor.id()},
        "key":maximum.key(),
        "schema":{"id":descriptor.value_schema(),"version":descriptor.schema_version()},
        "value":maximum,
    });
    canonical_json_bytes_with_limit(&document, descriptor.max_record_bytes())
        .map(|_| ())
        .map_err(|_| PublicationError::Capacity)
}

#[cfg(test)]
#[path = "admission_capacity_tests.rs"]
mod tests;

//! Fixed Core profile descriptors; structural codecs never mint a proof.
use crate::commerce::records::Record;
use mush_core::{
    CommitToken, ModError, Registrar, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
    mechanics::CanonicalId,
};

pub(crate) fn register<R: Record>(
    reg: &mut Registrar<'_>,
) -> Result<StateFamilyHandle<String, R>, ModError> {
    let error = |e: String| ModError::msg(e);
    let id = |suffix: &str| {
        CanonicalId::new(format!("{}.{}.v1", R::FAMILY, suffix)).map_err(|e| error(e.to_string()))
    };
    let descriptor = StateFamilyDescriptor::new(
        StateFamilyId::new(R::FAMILY).map_err(|e| error(e.to_string()))?,
        id("key")?,
        id("value")?,
        1,
        id("decode")?,
        id("validate")?,
        id("canonical")?,
    )
    .and_then(|d| d.with_limits(32, R::MAX_BYTES))
    .map_err(|e| error(e.to_string()))?;
    reg.state_family(
        descriptor,
        |key: &String| {
            let op = CommitToken::from_hex(key).map_err(|e| e.to_string())?;
            if op.to_hex() == *key {
                Ok(key.clone())
            } else {
                Err("noncanonical Conduct key".into())
            }
        },
        |value: &serde_json::Value| {
            let row: R = serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
            row.validate().map_err(|e| e.to_string())?;
            Ok(row)
        },
        |row: &R| row.validate().map_err(|e| e.to_string()),
        |row: &R| serde_json::to_value(row).map_err(|e| e.to_string()),
    )
}

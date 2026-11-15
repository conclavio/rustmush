//! Exact kernel-recognized Market families. E4 owns the permanent offer cap;
//! adding a third progress counter would violate Fund's closed two-write set.
use super::Issuer;
use super::config::RenewalConfig;
use super::records::Row;
use mush_core::mechanics::CanonicalId;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{
    CommitToken, ModError, Registrar, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
};

pub(crate) struct Families<I: Issuer> {
    pub(crate) operations: StateFamilyHandle<CommitToken, I::Renewal>,
    pub(crate) receipts: StateFamilyHandle<CommitToken, I::Receipt>,
}
impl<I: Issuer> Clone for Families<I> {
    fn clone(&self) -> Self {
        Self {
            operations: self.operations.clone(),
            receipts: self.receipts.clone(),
        }
    }
}
pub(crate) fn family<R: Row>(
    reg: &mut Registrar<'_>,
) -> Result<StateFamilyHandle<CommitToken, R>, ModError> {
    let id = |suffix| {
        CanonicalId::new(format!("{}.{suffix}.v1", R::FAMILY)).map_err(super::config::error)
    };
    let descriptor = StateFamilyDescriptor::new(
        StateFamilyId::new(R::FAMILY).map_err(super::config::error)?,
        id("key")?,
        id("value")?,
        1,
        id("decode")?,
        id("validate")?,
        id("canonical")?,
    )
    .and_then(|d| d.with_limits(32, R::MAX_BYTES))
    .map_err(super::config::error)?;
    reg.state_family(
        descriptor,
        |k: &CommitToken| Ok(k.to_hex()),
        |v: &serde_json::Value| {
            let bytes = mush_core::canonical_json_bytes_with_limit(v, R::MAX_BYTES)
                .map_err(|e| e.to_string())?;
            let row: R = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            if serde_json::to_value(&row).map_err(|e| e.to_string())? != *v {
                return Err(super::records::invalid());
            }
            row.validate()?;
            Ok(row)
        },
        R::validate,
        |row: &R| serde_json::to_value(row).map_err(|e| e.to_string()),
    )
}
impl<I: Issuer> Families<I> {
    pub(crate) fn register(
        reg: &mut Registrar<'_>,
        config: &RenewalConfig<I>,
    ) -> Result<(Self, OwnerReadHandle<I::Reader>), ModError> {
        // Exactly one owner-read request is shared by all five typed consumers.
        Ok((
            Self {
                operations: family(reg)?,
                receipts: family(reg)?,
            },
            I::request_reader(reg, &config.manifest, config.service.is_some())?,
        ))
    }
}

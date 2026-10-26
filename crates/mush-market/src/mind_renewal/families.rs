//! Exact kernel-recognized Market families. E4 owns the permanent offer cap;
//! adding a third progress counter would violate Fund's closed two-write set.
use super::{MindRenewalOperationV1, MindRenewalReceiptV1, RenewalConfig};
use mush_core::mechanics::CanonicalId;
use mush_core::read_service::OwnerReadHandle;
use mush_core::{
    CommitToken, ModError, Registrar, StateFamilyDescriptor, StateFamilyHandle, StateFamilyId,
};
use mush_roll::commerce::renewal::reader::RenewalReader;
use serde::{Serialize, de::DeserializeOwned};

pub(super) trait Row: Clone + Serialize + DeserializeOwned + Send + Sync + 'static {
    const FAMILY: &'static str;
    const MAX_BYTES: usize;
    fn validate(&self) -> Result<(), String>;
    fn key(&self) -> String;
}
impl Row for MindRenewalOperationV1 {
    const FAMILY: &'static str = MindRenewalOperationV1::FAMILY;
    const MAX_BYTES: usize = MindRenewalOperationV1::MAX_BYTES;
    fn validate(&self) -> Result<(), String> {
        Self::validate(self)
    }
    fn key(&self) -> String {
        Self::key(self)
    }
}
impl Row for MindRenewalReceiptV1 {
    const FAMILY: &'static str = MindRenewalReceiptV1::FAMILY;
    const MAX_BYTES: usize = MindRenewalReceiptV1::MAX_BYTES;
    fn validate(&self) -> Result<(), String> {
        Self::validate(self)
    }
    fn key(&self) -> String {
        self.operation().to_hex()
    }
}
pub(crate) struct Families {
    pub(super) operations: StateFamilyHandle<CommitToken, MindRenewalOperationV1>,
    pub(super) receipts: StateFamilyHandle<CommitToken, MindRenewalReceiptV1>,
}
impl Clone for Families {
    fn clone(&self) -> Self {
        Self {
            operations: self.operations.clone(),
            receipts: self.receipts.clone(),
        }
    }
}
pub(super) fn family<R: Row>(
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
impl Families {
    pub(super) fn register(
        reg: &mut Registrar<'_>,
        config: &RenewalConfig,
    ) -> Result<(Self, OwnerReadHandle<RenewalReader>), ModError> {
        // Exactly one owner-read request is shared by all five typed consumers.
        Ok((
            Self {
                operations: family(reg)?,
                receipts: family(reg)?,
            },
            RenewalReader::request_configured(reg, &config.manifest, config.service.is_some())?,
        ))
    }
}

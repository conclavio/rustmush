//! Optional exact physical input for the closed Industry core-integration path.
//!
//! Inventory proves quantity, identity and K reservation history. Certification,
//! payment and the independent order inventory remain the actual Industry/E4
//! owners' obligations; these records cannot substitute for those proofs.
use mush_core::{ActionTxn, CanonicalId, CommitToken, DefinitionDigest, DefinitionRef};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

pub(crate) mod audit;
mod records;
pub(crate) mod registration;
mod writer;
pub use records::{CoreInputIdentity, CoreInputOperation, CoreInputState};
pub use registration::{CoreInputHandle, RegisteredCoreInputs};
pub use writer::{CoreInputRequest, CoreInputWitness};
#[cfg(test)]
mod tests;

pub const ACCEPT_ACTION: &str = "accept_core_integration";
pub const COMPLETE_ACTION: &str = "complete_core_integration";
pub const CANCEL_ACTION: &str = "cancel_core_integration";
pub const MAX_OPERATIONS: u64 = 8192;
pub const MAX_OPERATION_BYTES: usize = 12 * 1024;
pub const MAX_ENVELOPE_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("exact core input identity or history disagrees")]
    Integrity,
    #[error("exact core input authority is unavailable")]
    Unavailable,
    #[error("exact core input is held or retired")]
    Reserved,
    #[error("exact core input capacity is exhausted")]
    Capacity,
}
fn bad<T>(_: T) -> Error {
    Error::Integrity
}
fn cid(text: &str) -> Result<CanonicalId, Error> {
    CanonicalId::new(text).map_err(bad)
}
fn bytes(v: &Value, limit: usize) -> Result<Vec<u8>, Error> {
    mush_core::mechanics::canonical_json_bytes_with_limit(v, limit).map_err(|_| Error::Capacity)
}
fn encode<T: Serialize>(r: &T) -> Result<Value, String> {
    let v = serde_json::to_value(r).map_err(|_| "exact core input encoding".to_owned())?;
    bytes(&v, MAX_OPERATION_BYTES).map_err(|e| e.to_string())?;
    Ok(v)
}
fn decode<T: DeserializeOwned + Serialize>(v: &Value) -> Result<T, String> {
    bytes(v, MAX_OPERATION_BYTES).map_err(|e| e.to_string())?;
    let r: T =
        serde_json::from_value(v.clone()).map_err(|_| "exact core input decoding".to_owned())?;
    if serde_json::to_value(&r).map_err(|_| "exact core input encoding".to_owned())? != *v {
        return Err("noncanonical exact core input".into());
    }
    Ok(r)
}
fn digest(domain: &str, v: &impl Serialize) -> Result<DefinitionDigest, Error> {
    DefinitionDigest::for_canonical_value_with_limit(
        domain,
        &encode(v).map_err(bad)?,
        MAX_OPERATION_BYTES,
    )
    .map_err(bad)
}

/// Trusted composition chooses the exact three existing Industry actions.
/// No command or durable row can create a registered input handle.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreInputBinding {
    accept: DefinitionRef,
    complete: DefinitionRef,
    cancel: DefinitionRef,
}
impl CoreInputBinding {
    pub fn new(
        accept: DefinitionRef,
        complete: DefinitionRef,
        cancel: DefinitionRef,
    ) -> Result<Self, Error> {
        let value = Self {
            accept,
            complete,
            cancel,
        };
        value.validate()?;
        Ok(value)
    }
    pub fn actions(&self) -> [&DefinitionRef; 3] {
        [&self.accept, &self.complete, &self.cancel]
    }
    fn validate(&self) -> Result<(), Error> {
        for (r, id) in
            self.actions()
                .into_iter()
                .zip([ACCEPT_ACTION, COMPLETE_ACTION, CANCEL_ACTION])
        {
            if r.key().owner.as_str() != "industry"
                || r.key().kind.as_str() != "action"
                || r.key().id.as_str() != id
                || r.version() != 1
            {
                return Err(Error::Integrity);
            }
        }
        Ok(())
    }
}

impl crate::InventoryFamilies {
    /// Request the configured Inventory read capability during actual Industry
    /// registration. Ordinary quantity delegates do not acquire this capability.
    pub fn core_inputs(&self) -> Option<&CoreInputHandle> {
        self.core_inputs.as_ref()
    }

    pub(crate) fn require_unheld_lot(
        &self,
        txn: &mut ActionTxn<'_>,
        id: crate::LotId,
    ) -> Result<(), crate::InventoryError> {
        self.core_inputs
            .as_ref()
            .map_or(Ok(()), |h| h.require_unheld(txn, id))
            .map_err(|_| crate::InventoryError::ExactInput)
    }
    pub(crate) fn require_new_lot(
        &self,
        txn: &mut ActionTxn<'_>,
        id: crate::LotId,
    ) -> Result<(), crate::InventoryError> {
        self.core_inputs
            .as_ref()
            .map_or(Ok(()), |h| h.require_new(txn, id))
            .map_err(|_| crate::InventoryError::ExactInput)
    }
}

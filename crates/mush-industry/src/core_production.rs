//! Optional certified Origin stock producer. Commodity quality alone is not certification.
use mush_core::mechanics::canonical_json_bytes_with_limit;
use mush_core::{CanonicalId, DefinitionRef, ModError};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
/// An owner preparation's result: its output, its patch and the replay
/// writes the composed commit carries.
pub(crate) type Prepared<O, P> =
    Result<(Box<O>, Box<P>, Vec<mush_core::ReplayWrite>), mush_core::participant::ParticipantError>;
mod audit;
mod authority;
mod config;
pub mod integration;
mod participant;
mod provider;
mod records;
pub use config::{
    CoreOriginConfig, CoreOriginMode, OriginAgent, OriginPolicy, origin_content_descriptor,
};
pub(crate) use provider::Runtime;
pub use records::{CertifiedOrigin, DieCertificate, DieCertificates, DieClass};
#[cfg(test)]
mod tests;

pub const ISSUE_ACTION: &str = "issue_core_die";
pub const INITIALIZE_ACTION: &str = "initialize_core_origin";
pub const IMPORT_CAPABILITY: &str = "industry.core_die.issue";
pub(crate) const INDEX_KEY: &str = "core_origin_registry";
pub(crate) const SOURCE_KIND: &str = "certified_core_die.v1";
pub(crate) const MAX_CERTIFICATES: u64 = 8192;
pub(crate) const RECORD_BYTES: usize = 8192;
#[derive(Debug, Clone, Copy, Eq, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("certified Origin request is invalid")]
    Invalid,
    #[error("certified Origin history disagrees")]
    Integrity,
    #[error("certified Origin capacity is exhausted")]
    Capacity,
    #[error("certified Origin authority is unavailable")]
    Unavailable,
}
pub(crate) fn bad<T>(_: T) -> Error {
    Error::Integrity
}
pub(crate) fn mod_error(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn id(s: &str) -> Result<CanonicalId, Error> {
    CanonicalId::new(s).map_err(bad)
}
pub(crate) fn encode<T: Serialize>(r: &T) -> Result<Value, String> {
    let v = serde_json::to_value(r).map_err(|e| e.to_string())?;
    canonical_json_bytes_with_limit(&v, RECORD_BYTES).map_err(|e| e.to_string())?;
    Ok(v)
}
pub(crate) fn decode<T: DeserializeOwned + Serialize>(v: &Value) -> Result<T, String> {
    canonical_json_bytes_with_limit(v, RECORD_BYTES).map_err(|e| e.to_string())?;
    let r: T = serde_json::from_value(v.clone()).map_err(|e| e.to_string())?;
    if serde_json::to_value(&r).map_err(|e| e.to_string())? != *v {
        return Err("noncanonical certified Origin record".into());
    }
    Ok(r)
}
pub(crate) fn fixed(r: &DefinitionRef, kind: &str, name: &str) -> bool {
    r.key().owner.as_str() == crate::NS
        && r.key().kind.as_str() == kind
        && r.key().id.as_str() == name
        && r.version() == 1
}

pub mod refit;

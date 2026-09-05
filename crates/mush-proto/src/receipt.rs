//! Bounded player-facing authoritative-action status and result vocabulary.
//!
//! These types deliberately contain no definition digest, truth identifier,
//! request/plan/bundle hash, guard, or persistence detail. The game server
//! projects a compact authoritative receipt into this public shape before it
//! crosses either gateway boundary.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use smol_str::SmolStr;

use crate::{ViewEnvelope, ViewEnvelopeError};

/// Maximum bytes in the self-contained public status reference.
pub const MAX_ACTION_STATUS_ID_BYTES: usize = 256;
/// Maximum bytes in a projected result's human-readable summary.
pub const MAX_PROJECTED_ACTION_SUMMARY_BYTES: usize = 4 * 1024;
/// Maximum JSON bytes in a projected result payload.
pub const MAX_PROJECTED_ACTION_RESULT_BYTES: usize = 64 * 1024;

/// A public, stable authoritative commit token carried by projected receipts.
///
/// This type is intentionally narrower than the internal transaction identity:
/// clients may retain, compare, and acknowledge it, but cannot reconstruct a
/// request, plan, bundle, or outcome hash from it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ProjectedCommitToken(SmolStr);

impl ProjectedCommitToken {
    pub fn new(value: impl Into<SmolStr>) -> Result<Self, ProjectedCommitTokenError> {
        let value = value.into();
        if !is_lower_hex(&value, 32) {
            return Err(ProjectedCommitTokenError);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProjectedCommitToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for ProjectedCommitToken {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ProjectedCommitToken {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

/// Invalid projected commit token.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
#[error("projected commit token is not 32 lowercase hexadecimal digits")]
pub struct ProjectedCommitTokenError;

/// A public, self-contained reference for authorized `action/status` lookup.
///
/// Its fields identify only the authenticated caller's idempotency scope and
/// request. It carries no ingress, plan, bundle, or outcome hash. The server
/// reconstructs the principal from the live transport and refuses a lookup
/// whose scope does not belong to that principal.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ActionStatusId(SmolStr);

impl ActionStatusId {
    pub fn new(value: impl Into<SmolStr>) -> Result<Self, ActionStatusIdError> {
        let value = value.into();
        validate_status_id(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ActionStatusId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl Serialize for ActionStatusId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for ActionStatusId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

fn validate_status_id(value: &str) -> Result<(), ActionStatusIdError> {
    if value.is_empty() {
        return Err(ActionStatusIdError::Empty);
    }
    if value.len() > MAX_ACTION_STATUS_ID_BYTES {
        return Err(ActionStatusIdError::TooLong {
            actual: value.len(),
            max: MAX_ACTION_STATUS_ID_BYTES,
        });
    }
    if let Some((offset, byte)) = value
        .bytes()
        .enumerate()
        .find(|(_, byte)| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b':'))
    {
        return Err(ActionStatusIdError::InvalidByte { offset, byte });
    }
    let mut parts = value.split(':');
    let prefix = parts.next();
    let owner = parts.next();
    let namespace = parts.next();
    let epoch = parts.next();
    let request = parts.next();
    if prefix != Some("k5")
        || !owner.is_some_and(is_canonical_id)
        || !namespace.is_some_and(is_canonical_id)
        || !epoch.is_some_and(|part| is_lower_hex(part, 16))
        || !request.is_some_and(|part| is_lower_hex(part, 32))
        || parts.next().is_some()
    {
        return Err(ActionStatusIdError::InvalidShape);
    }
    Ok(())
}

fn is_canonical_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && !value.starts_with('.')
        && !value.ends_with('.')
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.'))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

/// Invalid public action-status identity.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ActionStatusIdError {
    #[error("action status reference is empty")]
    Empty,
    #[error("action status reference is {actual} bytes; maximum is {max}")]
    TooLong { actual: usize, max: usize },
    #[error("action status reference contains invalid byte {byte:#04x} at byte {offset}")]
    InvalidByte { offset: usize, byte: u8 },
    #[error("action status reference has an invalid shape")]
    InvalidShape,
}

/// Provider-owned, already-redacted semantic result.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ProjectedActionResultWire")]
pub struct ProjectedActionResult {
    kind: SmolStr,
    summary: String,
    data: Value,
}

#[derive(Deserialize)]
struct ProjectedActionResultWire {
    kind: SmolStr,
    summary: String,
    data: Value,
}

impl ProjectedActionResult {
    pub fn new(
        kind: impl Into<SmolStr>,
        summary: impl Into<String>,
        data: Value,
    ) -> Result<Self, ProjectedActionResultError> {
        let result = Self {
            kind: kind.into(),
            summary: summary.into(),
            data,
        };
        result.validate()?;
        Ok(result)
    }

    fn validate(&self) -> Result<(), ProjectedActionResultError> {
        if !is_canonical_id(&self.kind) {
            return Err(ProjectedActionResultError::InvalidKind);
        }
        if self.summary.is_empty() {
            return Err(ProjectedActionResultError::EmptySummary);
        }
        if self.summary.chars().any(char::is_control) {
            return Err(ProjectedActionResultError::InvalidSummaryText);
        }
        if self.summary.len() > MAX_PROJECTED_ACTION_SUMMARY_BYTES {
            return Err(ProjectedActionResultError::SummaryTooLong {
                actual: self.summary.len(),
                max: MAX_PROJECTED_ACTION_SUMMARY_BYTES,
            });
        }
        let bytes = serde_json::to_vec(&self.data)
            .map_err(|_| ProjectedActionResultError::DataSerialization)?
            .len();
        if bytes > MAX_PROJECTED_ACTION_RESULT_BYTES {
            return Err(ProjectedActionResultError::DataTooLarge {
                actual: bytes,
                max: MAX_PROJECTED_ACTION_RESULT_BYTES,
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn kind(&self) -> &str {
        &self.kind
    }

    #[must_use]
    pub fn summary(&self) -> &str {
        &self.summary
    }

    #[must_use]
    pub const fn data(&self) -> &Value {
        &self.data
    }
}

impl TryFrom<ProjectedActionResultWire> for ProjectedActionResult {
    type Error = ProjectedActionResultError;

    fn try_from(value: ProjectedActionResultWire) -> Result<Self, Self::Error> {
        Self::new(value.kind, value.summary, value.data)
    }
}

/// Invalid public result projection.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProjectedActionResultError {
    #[error("projected action result kind is not canonical")]
    InvalidKind,
    #[error("projected action result summary is empty")]
    EmptySummary,
    #[error("projected action result summary contains a control character")]
    InvalidSummaryText,
    #[error("projected action result summary is {actual} bytes; maximum is {max}")]
    SummaryTooLong { actual: usize, max: usize },
    #[error("projected action result data cannot be serialized")]
    DataSerialization,
    #[error("projected action result data is {actual} bytes; maximum is {max}")]
    DataTooLarge { actual: usize, max: usize },
}

/// A committed authoritative result safe for one eligible viewer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ProjectedActionReceiptWire")]
pub struct ProjectedActionReceipt {
    schema_version: u16,
    status: ActionStatusId,
    commit: SmolStr,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    result: Option<Box<ProjectedActionResult>>,
}

#[derive(Deserialize)]
struct ProjectedActionReceiptWire {
    schema_version: u16,
    status: ActionStatusId,
    commit: SmolStr,
    #[serde(default)]
    result: Option<Box<ProjectedActionResult>>,
}

impl ProjectedActionReceipt {
    pub const SCHEMA_VERSION: u16 = 1;

    pub fn new(
        status: ActionStatusId,
        commit: impl Into<SmolStr>,
        result: Option<ProjectedActionResult>,
    ) -> Result<Self, ProjectedActionReceiptError> {
        let receipt = Self {
            schema_version: Self::SCHEMA_VERSION,
            status,
            commit: commit.into(),
            result: result.map(Box::new),
        };
        receipt.validate()?;
        Ok(receipt)
    }

    fn validate(&self) -> Result<(), ProjectedActionReceiptError> {
        if self.schema_version != Self::SCHEMA_VERSION {
            return Err(ProjectedActionReceiptError::UnsupportedSchema {
                actual: self.schema_version,
                supported: Self::SCHEMA_VERSION,
            });
        }
        if !is_lower_hex(&self.commit, 32) {
            return Err(ProjectedActionReceiptError::InvalidCommit);
        }
        Ok(())
    }

    #[must_use]
    pub const fn status(&self) -> &ActionStatusId {
        &self.status
    }

    #[must_use]
    pub fn commit(&self) -> &str {
        &self.commit
    }

    #[must_use]
    pub fn result(&self) -> Option<&ProjectedActionResult> {
        self.result.as_deref()
    }

    /// Deterministic restart-safe result keyframe. Every commit is a fresh
    /// replaceable generation and sequence 1; duplicate delivery therefore
    /// carries the same commit token and frame identity.
    pub fn view(&self) -> Result<ViewEnvelope, ViewEnvelopeError> {
        let generation = u64::from_str_radix(&self.commit[..13], 16)
            .expect("validated lowercase commit prefix is hexadecimal")
            .max(1);
        let data = serde_json::to_value(self)
            .expect("projected action receipts contain only serializable fields");
        ViewEnvelope::keyframe("resolution.outcome", 1, generation, 1, data)
            .and_then(|view| view.with_commit(self.commit.to_string()))
    }
}

impl TryFrom<ProjectedActionReceiptWire> for ProjectedActionReceipt {
    type Error = ProjectedActionReceiptError;

    fn try_from(value: ProjectedActionReceiptWire) -> Result<Self, Self::Error> {
        let receipt = Self {
            schema_version: value.schema_version,
            status: value.status,
            commit: value.commit,
            result: value.result,
        };
        receipt.validate()?;
        Ok(receipt)
    }
}

/// Invalid player-facing committed receipt.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum ProjectedActionReceiptError {
    #[error("unsupported projected action receipt schema {actual}; supported is {supported}")]
    UnsupportedSchema { actual: u16, supported: u16 },
    #[error("projected action receipt commit is not 32 lowercase hexadecimal digits")]
    InvalidCommit,
}

/// Authorized semantic status projected without persistence internals.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "detail", rename_all = "snake_case")]
pub enum ProjectedActionStatus {
    Unknown,
    Expired,
    Pending { reference: ActionStatusId },
    Committed(ProjectedActionReceipt),
    Unavailable { reference: ActionStatusId },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> ActionStatusId {
        ActionStatusId::new("k5:toy:brace:0000000000000007:01010101010101010101010101010101")
            .unwrap()
    }

    #[test]
    fn status_id_and_receipt_reject_noncanonical_wire_shapes() {
        assert!(ActionStatusId::new("k5:Toy:brace:0000000000000007:0101").is_err());
        assert!(
            serde_json::from_value::<ProjectedActionReceipt>(serde_json::json!({
                "schema_version": 1,
                "status": status(),
                "commit": "ABCDEFABCDEFABCDEFABCDEFABCDEFAB"
            }))
            .is_err()
        );
        assert!(
            ProjectedActionResult::new("brace", "committed\nforged", serde_json::json!({}))
                .is_err()
        );
        assert!(
            ProjectedActionResult::new(
                "brace",
                "committed",
                serde_json::json!({"oversized": "x".repeat(MAX_PROJECTED_ACTION_RESULT_BYTES)}),
            )
            .is_err()
        );
    }

    #[test]
    fn committed_receipt_builds_a_deterministic_deduplicated_keyframe() {
        let result = ProjectedActionResult::new(
            "brace",
            "Brace committed.",
            serde_json::json!({"focus": 2}),
        )
        .unwrap();
        let receipt =
            ProjectedActionReceipt::new(status(), "0123456789abcdef0123456789abcdef", Some(result))
                .unwrap();
        let first = receipt.view().unwrap();
        let replay = receipt.view().unwrap();
        assert_eq!(first, replay);
        assert_eq!(first.view(), "resolution.outcome");
        assert_eq!(first.sequence(), 1);
        assert_eq!(first.commit(), Some(receipt.commit()));
    }
}

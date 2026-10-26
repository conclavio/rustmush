//! Immutable filing content structure, not a filed right or certified grant.
//!
//! This value is only a component of a future complete typed filing record.
//! It carries no rating, fee/title receipt, withdrawal/recall authority, holder,
//! source bytes or permission. No family or endpoint is registered here. A
//! successful link check proves agreement with the supplied lookup, not that
//! the lookup is committed, complete, authorized or source-audited. The real
//! composed owner/boot audit must establish those properties separately.

use std::{collections::BTreeSet, num::NonZeroU64};

use mush_core::canonical_json_bytes_with_limit;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::{CharterId, FiledRevisionId, LineId, RevisionIdentity, WorldIcSeconds};
use crate::{Revision, RevisionKey};

/// Version of this structural component, independent of workflow JSON.
pub const CONTENT_FORMAT: u16 = 1;
/// Own content plus at most three actual parents; no recursive serialized tree.
pub const MAX_PARENT_DEPTH: usize = 3;
/// Fixed identity/scalar fields and one immediate parent reference fit in 4 KiB.
/// This is a decoder preflight and canonical encoder cap, not a family quota.
pub const MAX_CONTENT_BYTES: usize = 4 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContentError {
    #[error("unsupported filing-content version")]
    Version,
    #[error("invalid bounded filing-content encoding")]
    Encoding,
    #[error("filing-content key does not match its identity")]
    Key,
    #[error("invalid filing sequence")]
    Sequence,
    #[error("filing ancestry is cyclic or exceeds the bound")]
    Ancestry,
    #[error("filing-content lookup is unavailable")]
    Unavailable,
    #[error("filing-content reference is missing")]
    Missing,
    #[error("filing-content reference does not match")]
    Mismatch,
}

/// A nonzero sequence within one compatible line, not an allocator capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FilingSequence(NonZeroU64);

impl FilingSequence {
    pub fn new(value: u64) -> Result<Self, ContentError> {
        NonZeroU64::new(value)
            .map(Self)
            .ok_or(ContentError::Sequence)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl Serialize for FilingSequence {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.get().to_string())
    }
}

impl<'de> Deserialize<'de> for FilingSequence {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visitor;
        impl serde::de::Visitor<'_> for Visitor {
            type Value = FilingSequence;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a nonzero canonical decimal filing sequence")
            }

            fn visit_str<E: serde::de::Error>(self, text: &str) -> Result<Self::Value, E> {
                if text.is_empty()
                    || text.len() > 20
                    || text.starts_with('0')
                    || !text.bytes().all(|c| c.is_ascii_digit())
                {
                    return Err(E::custom(ContentError::Sequence));
                }
                let number = text
                    .parse()
                    .map_err(|_| E::custom(ContentError::Sequence))?;
                FilingSequence::new(number).map_err(E::custom)
            }
        }
        deserializer.deserialize_str(Visitor)
    }
}

/// Exact immediate source ancestry. It implies no qualifying parent licence,
/// royalty terms, source access, certification or entitlement continuation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParentContent {
    pub filed: FiledRevisionId,
    pub revision: RevisionIdentity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilingPosition {
    pub charter: CharterId,
    pub line: LineId,
    pub sequence: FilingSequence,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "ContentWire")]
pub struct FiledRevisionContent {
    format: u16,
    filed: FiledRevisionId,
    position: FilingPosition,
    revision: RevisionIdentity,
    filed_at: WorldIcSeconds,
    parent: Option<ParentContent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentWire {
    format: u16,
    filed: FiledRevisionId,
    position: FilingPosition,
    revision: RevisionIdentity,
    filed_at: WorldIcSeconds,
    // Require the explicit null for a root; omission must not silently erase
    // immutable ancestry during record decoding.
    #[serde(deserialize_with = "required_parent")]
    parent: Option<ParentContent>,
}

fn required_parent<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<ParentContent>, D::Error> {
    Option::<ParentContent>::deserialize(deserializer)
}

impl TryFrom<ContentWire> for FiledRevisionContent {
    type Error = ContentError;

    fn try_from(wire: ContentWire) -> Result<Self, Self::Error> {
        if wire.format != CONTENT_FORMAT {
            return Err(ContentError::Version);
        }
        if wire.parent.is_some_and(|parent| parent.filed == wire.filed) {
            return Err(ContentError::Ancestry);
        }
        Ok(Self {
            format: wire.format,
            filed: wire.filed,
            position: wire.position,
            revision: wire.revision,
            filed_at: wire.filed_at,
            parent: wire.parent,
        })
    }
}

impl FiledRevisionContent {
    /// Capture structurally valid saved content. No caller authority is inferred.
    pub fn capture(
        filed: FiledRevisionId,
        position: FilingPosition,
        revision: &Revision,
        filed_at: WorldIcSeconds,
        parent: Option<ParentContent>,
    ) -> Result<Self, ContentError> {
        Self::try_from(ContentWire {
            format: CONTENT_FORMAT,
            filed,
            position,
            revision: RevisionIdentity::capture(revision).map_err(|_| ContentError::Mismatch)?,
            filed_at,
            parent,
        })
    }

    #[must_use]
    pub const fn id(&self) -> FiledRevisionId {
        self.filed
    }

    #[must_use]
    pub const fn position(&self) -> FilingPosition {
        self.position
    }

    #[must_use]
    pub const fn revision(&self) -> RevisionIdentity {
        self.revision
    }

    #[must_use]
    pub const fn filed_at(&self) -> WorldIcSeconds {
        self.filed_at
    }

    #[must_use]
    pub const fn parent(&self) -> Option<ParentContent> {
        self.parent
    }

    /// Reuse the filed identity's one canonical key; no root-side allocator.
    #[must_use]
    pub fn key(&self) -> String {
        self.filed.key()
    }

    pub fn encode(&self) -> Result<Vec<u8>, ContentError> {
        let value = serde_json::to_value(self).map_err(|_| ContentError::Encoding)?;
        canonical_json_bytes_with_limit(&value, MAX_CONTENT_BYTES)
            .map_err(|_| ContentError::Encoding)
    }

    /// Bound raw bytes before JSON allocation, then check the exact key. The
    /// complete future family must independently enforce its aggregate quotas.
    pub fn decode(key: &str, bytes: &[u8]) -> Result<Self, ContentError> {
        if bytes.len() > MAX_CONTENT_BYTES {
            return Err(ContentError::Encoding);
        }
        let wire: ContentWire =
            serde_json::from_slice(bytes).map_err(|_| ContentError::Encoding)?;
        let value = Self::try_from(wire)?;
        if key != value.key() {
            return Err(ContentError::Key);
        }
        Ok(value)
    }

    /// Validate only structural links in the supplied available snapshot.
    /// This returns no retention capability and cannot certify/activate filing.
    pub fn check_links<L: ContentLookup>(&self, lookup: &L) -> Result<(), ContentError> {
        // Even a parentless root requires an explicitly available content view;
        // absent/unloaded family state cannot masquerade as valid empty ancestry.
        lookup.check_available()?;
        let mut seen = BTreeSet::from([self.filed]);
        let mut current = self;
        let mut depth = 0;
        loop {
            let charter = lookup
                .line_charter(current.position.line)?
                .ok_or(ContentError::Missing)?;
            if charter != current.position.charter {
                return Err(ContentError::Mismatch);
            }
            let revision = lookup
                .revision(current.revision.revision().key)?
                .ok_or(ContentError::Missing)?;
            if !current
                .revision
                .matches(revision)
                .map_err(|_| ContentError::Mismatch)?
            {
                return Err(ContentError::Mismatch);
            }
            let Some(parent) = current.parent else {
                return Ok(());
            };
            depth += 1;
            if depth > MAX_PARENT_DEPTH || !seen.insert(parent.filed) {
                return Err(ContentError::Ancestry);
            }
            let record = lookup.filed(parent.filed)?.ok_or(ContentError::Missing)?;
            if record.filed != parent.filed
                || record.revision != parent.revision
                || record.filed_at > current.filed_at
            {
                return Err(ContentError::Mismatch);
            }
            current = record;
        }
    }
}

/// Read-only structural input. The caller must later prove it is a complete
/// committed snapshot with current source/integrity audits. No implementation
/// is installed by this module, and this trait grants no state mutation access.
pub trait ContentLookup {
    /// Return Unavailable if any required family/source view is not ready.
    fn check_available(&self) -> Result<(), ContentError>;
    fn line_charter(&self, line: LineId) -> Result<Option<CharterId>, ContentError>;
    fn revision(&self, key: RevisionKey) -> Result<Option<&Revision>, ContentError>;
    fn filed(&self, filed: FiledRevisionId) -> Result<Option<&FiledRevisionContent>, ContentError>;
}

#[cfg(test)]
mod tests;

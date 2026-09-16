//! **The apply latch** (plan §3.1): the durable record that a pack was applied, entry
//! by entry — which object each entry made and which revision it was made from.
//!
//! The latch is what makes apply idempotent and retire safe: [`crate::apply::plan`]
//! diffs a pack against its latch to find what to create, update or retire, and an
//! unchanged pack applied twice plans nothing. It generalizes `tacsim`'s
//! `SeedApplication` — the same "objects and the latch or neither" invariant, kept by
//! writing the latch in the same batch as the objects — from a create-only census to a
//! per-entry ledger with revisions.
//!
//! A retired pack keeps its latch, marked `retired`, so a first-boot apply never
//! resurrects content staff removed on purpose: re-applying a retired pack is an
//! explicit staff act.

use std::collections::BTreeMap;

use mush_core::{CanonicalId, ClockStamp, DefinitionDigest, ObjId};
use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::pack::{LoadedPack, MAX_PACK_ENTRIES};

/// The persistence namespace: every latch lives at `mod:content:<key>`.
pub const NS: &str = "content";

/// The latch key for one pack. One latch per pack id — versions supersede each other
/// in place, because a pack's objects are its objects whatever version made them.
#[must_use]
pub fn applied_key(pack: &CanonicalId) -> SmolStr {
    SmolStr::new(format!("applied:{pack}"))
}

/// What one applied entry became.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(into = "AppliedEntryWire")]
pub struct AppliedEntry {
    kind: CanonicalId,
    digest: DefinitionDigest,
    object: Option<ObjId>,
}

/// The strict wire form of [`AppliedEntry`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedEntryWire {
    /// The entry kind at apply time.
    pub kind: CanonicalId,
    /// The entry digest at apply time — its revision.
    pub digest: String,
    /// The world object it made, if the kind makes one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object: Option<ObjId>,
}

impl AppliedEntry {
    /// One applied entry.
    #[must_use]
    pub fn new(kind: CanonicalId, digest: DefinitionDigest, object: Option<ObjId>) -> AppliedEntry {
        AppliedEntry {
            kind,
            digest,
            object,
        }
    }

    /// The entry kind.
    #[must_use]
    pub fn kind(&self) -> &CanonicalId {
        &self.kind
    }

    /// The revision applied.
    #[must_use]
    pub fn digest(&self) -> DefinitionDigest {
        self.digest
    }

    /// The object made, if any.
    #[must_use]
    pub fn object(&self) -> Option<ObjId> {
        self.object
    }
}

impl From<AppliedEntry> for AppliedEntryWire {
    fn from(value: AppliedEntry) -> AppliedEntryWire {
        AppliedEntryWire {
            kind: value.kind,
            digest: value.digest.to_hex(),
            object: value.object,
        }
    }
}

impl TryFrom<AppliedEntryWire> for AppliedEntry {
    type Error = LatchError;

    fn try_from(value: AppliedEntryWire) -> Result<AppliedEntry, LatchError> {
        let digest = DefinitionDigest::from_hex(&value.digest).map_err(|error| {
            LatchError::BadDigestText {
                detail: error.to_string(),
            }
        })?;
        Ok(AppliedEntry::new(value.kind, digest, value.object))
    }
}

impl<'de> Deserialize<'de> for AppliedEntry {
    fn deserialize<D>(deserializer: D) -> Result<AppliedEntry, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = AppliedEntryWire::deserialize(deserializer)?;
        AppliedEntry::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// The latch: one pack's applied state.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(into = "AppliedWire")]
pub struct Applied {
    pack: CanonicalId,
    version: u32,
    digest: DefinitionDigest,
    applied_at: ClockStamp,
    entries: BTreeMap<CanonicalId, AppliedEntry>,
    retired: bool,
    anomaly: Option<String>,
}

/// The strict wire form of [`Applied`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppliedWire {
    /// The pack.
    pub pack: CanonicalId,
    /// The version applied.
    pub version: u32,
    /// The pack digest applied.
    pub digest: String,
    /// When, on the world's IC clock.
    pub applied_at: ClockStamp,
    /// Every entry that is (or was) in the world, by pack-local id.
    pub entries: BTreeMap<CanonicalId, AppliedEntry>,
    /// Whether the pack was retired on purpose; a retired latch bars a boot re-apply.
    #[serde(default)]
    pub retired: bool,
    /// A paged diagnosis, if the last apply stopped part way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anomaly: Option<String>,
}

impl Applied {
    /// The one constructor.
    ///
    /// # Errors
    ///
    /// [`LatchError::TooManyEntries`] past [`MAX_PACK_ENTRIES`], and
    /// [`LatchError::ZeroVersion`].
    pub fn new(
        pack: CanonicalId,
        version: u32,
        digest: DefinitionDigest,
        applied_at: ClockStamp,
        entries: BTreeMap<CanonicalId, AppliedEntry>,
        retired: bool,
        anomaly: Option<String>,
    ) -> Result<Applied, LatchError> {
        if version == 0 {
            return Err(LatchError::ZeroVersion);
        }
        if entries.len() > MAX_PACK_ENTRIES {
            return Err(LatchError::TooManyEntries {
                actual: entries.len(),
                max: MAX_PACK_ENTRIES,
            });
        }
        Ok(Applied {
            pack,
            version,
            digest,
            applied_at,
            entries,
            retired,
            anomaly,
        })
    }

    /// The pack.
    #[must_use]
    pub fn pack(&self) -> &CanonicalId {
        &self.pack
    }

    /// The version applied.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The pack digest applied.
    #[must_use]
    pub fn digest(&self) -> DefinitionDigest {
        self.digest
    }

    /// When it was applied.
    #[must_use]
    pub fn applied_at(&self) -> &ClockStamp {
        &self.applied_at
    }

    /// Every applied entry, in id order.
    #[must_use]
    pub fn entries(&self) -> &BTreeMap<CanonicalId, AppliedEntry> {
        &self.entries
    }

    /// Whether the pack was retired on purpose.
    #[must_use]
    pub fn is_retired(&self) -> bool {
        self.retired
    }

    /// The paged diagnosis, if any.
    #[must_use]
    pub fn anomaly(&self) -> Option<&str> {
        self.anomaly.as_deref()
    }

    /// The pack-entry → object map (the seed latch's `placed`, generalized).
    #[must_use]
    pub fn placed(&self) -> BTreeMap<CanonicalId, ObjId> {
        self.entries
            .iter()
            .filter_map(|(id, e)| e.object.map(|o| (id.clone(), o)))
            .collect()
    }

    /// The object one entry made, if any.
    #[must_use]
    pub fn object_of(&self, entry: &CanonicalId) -> Option<ObjId> {
        self.entries.get(entry).and_then(AppliedEntry::object)
    }

    /// This latch as [`crate::check_requires`] sees it: a loaded pack at a digest.
    #[must_use]
    pub fn as_loaded(&self) -> ((CanonicalId, u32), LoadedPack) {
        (
            (self.pack.clone(), self.version),
            LoadedPack {
                digest: self.digest,
            },
        )
    }
}

impl From<Applied> for AppliedWire {
    fn from(value: Applied) -> AppliedWire {
        AppliedWire {
            pack: value.pack,
            version: value.version,
            digest: value.digest.to_hex(),
            applied_at: value.applied_at,
            entries: value.entries,
            retired: value.retired,
            anomaly: value.anomaly,
        }
    }
}

impl TryFrom<AppliedWire> for Applied {
    type Error = LatchError;

    fn try_from(value: AppliedWire) -> Result<Applied, LatchError> {
        let digest = DefinitionDigest::from_hex(&value.digest).map_err(|error| {
            LatchError::BadDigestText {
                detail: error.to_string(),
            }
        })?;
        Applied::new(
            value.pack,
            value.version,
            digest,
            value.applied_at,
            value.entries,
            value.retired,
            value.anomaly,
        )
    }
}

impl<'de> Deserialize<'de> for Applied {
    fn deserialize<D>(deserializer: D) -> Result<Applied, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = AppliedWire::deserialize(deserializer)?;
        Applied::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// Why a latch record was refused.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum LatchError {
    /// Version `0`.
    #[error("a pack version starts at one")]
    ZeroVersion,
    /// Past [`MAX_PACK_ENTRIES`].
    #[error("a latch records {actual} entries; the maximum is {max}")]
    TooManyEntries {
        /// Recorded.
        actual: usize,
        /// The cap.
        max: usize,
    },
    /// A digest that is not 64 hex digits.
    #[error("a latch digest is unreadable: {detail}")]
    BadDigestText {
        /// The reason.
        detail: String,
    },
}

/// The latches this world holds, keyed by pack — the content module's state slot.
#[derive(Clone, Debug, Default)]
pub struct Latches {
    applied: BTreeMap<CanonicalId, Applied>,
    /// Latch documents that would not decode, held by key so a bad document is
    /// neither silently dropped nor overwritten by a fresh apply.
    held: BTreeMap<SmolStr, String>,
}

impl Latches {
    /// Empty.
    #[must_use]
    pub fn new() -> Latches {
        Latches::default()
    }

    /// Decode every latch document from the module's boot data. A document that will
    /// not decode is **held** under its key with the reason, never dropped.
    #[must_use]
    pub fn load(kv: &[(SmolStr, serde_json::Value)]) -> Latches {
        let mut latches = Latches::new();
        for (key, value) in kv {
            let Some(_pack) = key.strip_prefix("applied:") else {
                continue;
            };
            match serde_json::from_value::<Applied>(value.clone()) {
                Ok(applied) => {
                    latches.applied.insert(applied.pack.clone(), applied);
                }
                Err(error) => {
                    tracing::warn!(key = %key, error = %error, "content: a latch document would not decode; held");
                    latches.held.insert(key.clone(), error.to_string());
                }
            }
        }
        latches
    }

    /// One pack's latch.
    #[must_use]
    pub fn applied(&self, pack: &CanonicalId) -> Option<&Applied> {
        self.applied.get(pack)
    }

    /// Every latch, in pack order.
    pub fn all(&self) -> impl Iterator<Item = &Applied> {
        self.applied.values()
    }

    /// Whether a latch document for `pack` is held undecodable.
    #[must_use]
    pub fn is_held(&self, pack: &CanonicalId) -> bool {
        self.held.contains_key(&applied_key(pack))
    }

    /// The held keys and their reasons.
    pub fn held(&self) -> impl Iterator<Item = (&SmolStr, &str)> {
        self.held.iter().map(|(k, v)| (k, v.as_str()))
    }

    /// Record a latch in RAM (the caller persists it in the same batch).
    pub fn set(&mut self, applied: Applied) {
        self.applied.insert(applied.pack.clone(), applied);
    }

    /// The applied, un-retired packs as [`crate::check_requires`] wants them.
    #[must_use]
    pub fn loaded(&self) -> BTreeMap<(CanonicalId, u32), LoadedPack> {
        self.applied
            .values()
            .filter(|a| !a.retired)
            .map(Applied::as_loaded)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::ClockRef;

    fn id(s: &str) -> CanonicalId {
        CanonicalId::new(s).expect("id")
    }

    fn stamp() -> ClockStamp {
        ClockStamp {
            clock: ClockRef::WorldIc { generation: 1 },
            at: 7,
        }
    }

    #[test]
    fn a_latch_round_trips_and_a_bad_document_is_held() {
        let mut entries = BTreeMap::new();
        entries.insert(
            id("room.a"),
            AppliedEntry::new(
                id("room"),
                DefinitionDigest::from_bytes([1; 32]),
                Some(ObjId(9)),
            ),
        );
        entries.insert(
            id("feature.wall"),
            AppliedEntry::new(id("feature"), DefinitionDigest::from_bytes([2; 32]), None),
        );
        let applied = Applied::new(
            id("lot"),
            1,
            DefinitionDigest::from_bytes([3; 32]),
            stamp(),
            entries,
            false,
            None,
        )
        .expect("latch");
        let value = serde_json::to_value(&applied).expect("encode");
        let back: Applied = serde_json::from_value(value.clone()).expect("decode");
        assert_eq!(back, applied);
        assert_eq!(back.placed().get(&id("room.a")), Some(&ObjId(9)));
        assert_eq!(back.object_of(&id("feature.wall")), None);

        let mut bad = value;
        if let Some(map) = bad.as_object_mut() {
            map.insert("digest".to_string(), serde_json::json!("nope"));
        }
        let latches = Latches::load(&[
            (
                applied_key(&id("lot")),
                serde_json::to_value(&applied).expect("v"),
            ),
            (applied_key(&id("broken")), bad),
            (SmolStr::new("unrelated:key"), serde_json::json!({})),
        ]);
        assert!(latches.applied(&id("lot")).is_some());
        assert!(latches.is_held(&id("broken")));
        assert_eq!(latches.loaded().len(), 1);
    }

    #[test]
    fn a_retired_latch_is_kept_but_not_loaded() {
        let applied = Applied::new(
            id("lot"),
            2,
            DefinitionDigest::from_bytes([3; 32]),
            stamp(),
            BTreeMap::new(),
            true,
            Some("retired by staff".into()),
        )
        .expect("latch");
        let mut latches = Latches::new();
        latches.set(applied);
        assert!(latches.applied(&id("lot")).is_some_and(Applied::is_retired));
        assert!(latches.loaded().is_empty());
        assert!(matches!(
            Applied::new(
                id("x"),
                0,
                DefinitionDigest::from_bytes([0; 32]),
                stamp(),
                BTreeMap::new(),
                false,
                None
            ),
            Err(LatchError::ZeroVersion)
        ));
    }
}

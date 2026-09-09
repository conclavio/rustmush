//! **Pack identity** (S1, doc 14 §2.5; plan §3.1).
//!
//! A pack is one JSON document: a [`PackManifest`] and a table of [`RawEntry`]s keyed
//! by their pack-local [`CanonicalId`]. This module knows nothing about what an entry
//! *means* — an owner registers the [`EntryKinds`] it decodes and does the decoding at
//! plan time — and everything about what a pack *is*: its bytes are strict, bounded and
//! deterministic, and its identity is a digest over its canonical encoding.
//!
//! # The digest
//!
//! [`pack_digest`] is a thin wrapper over
//! [`mush_core::DefinitionDigest::for_canonical_value_with_limit`] under
//! [`PACK_DIGEST_DOMAIN`] and [`MAX_PACK_BYTES`]: SHA-256 over the canonical JSON of
//! [`Pack::canonical_value`], the domain length-prefixed into the hash. There is no
//! second hash and no second canonical encoder here. What is hashed is
//!
//! ```text
//! { "manifest": { "id", "version", "kind", "requires" }, "entries": { id -> { "kind", "def" } } }
//! ```
//!
//! and nothing else — the manifest's own `digest` field is **not** inside it (a
//! self-referential hash is unverifiable), and the entries are hashed **as authored**,
//! canonically encoded: reordering keys or entries changes nothing, whitespace inside
//! a value changes everything, and no Rust type's serialization stands between the
//! file and its identity. That is the deliberate difference from `ruleset-tacsim`'s
//! typed pack, whose digest covers a re-serialization of typed entries; the two share
//! the primitive and part ways at the value (plan §3.1, deviation (a)).
//!
//! # Strict, bounded, deterministic
//!
//! Every wire type is `deny_unknown_fields` and re-enters through its own constructor,
//! so a cap is never advisory on load. Entries live in a [`BTreeMap`], so iteration is
//! id order and two loads of one file are byte-identical in memory. A repeated entry id
//! is an error, never last-write-wins. [`MAX_PACK_ENTRIES`], [`MAX_REQUIRES`] and
//! [`MAX_PACK_BYTES`] are re-checked by the wire.
//!
//! # No filesystem
//!
//! [`load_pack`] is `bytes -> Result<Pack, PackError>` and touches no file, so every
//! identity test is a unit test over a string and the host names the file when it
//! fails.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{CanonicalId, DefinitionDigest};
use serde::{Deserialize, Serialize};

/// The domain tag every content-pack digest is taken under.
///
/// `hash_parts` length-prefixes it, so this value *is* the encoding version: change the
/// pack encoding and this string changes with it, and every previously published
/// digest stops matching loudly rather than silently colliding.
pub const PACK_DIGEST_DOMAIN: &str = "content.pack.v1";

/// The domain tag one entry's digest is taken under (the per-entry revision the latch
/// diffs against; see `latch.rs`).
pub const ENTRY_DIGEST_DOMAIN: &str = "content.entry.v1";

/// The most entries one pack may declare.
pub const MAX_PACK_ENTRIES: usize = 1_024;

/// The most bytes one pack's canonical encoding may occupy.
///
/// Also the limit handed to the shared encoder, so an oversized pack is refused *by the
/// encoder* rather than hashed.
pub const MAX_PACK_BYTES: usize = 1_024 * 1_024;

/// The most bytes one entry's canonical encoding may occupy.
pub const MAX_ENTRY_BYTES: usize = 64 * 1_024;

/// The most dependencies one pack may declare.
pub const MAX_REQUIRES: usize = 16;

/// What a pack is for.
///
/// `RulesDefinition` and `CampaignSeed` are `ruleset-tacsim`'s two kinds, named here so
/// the encoding is one when its packs fold onto this crate (plan S7); the three atlas
/// kinds are S1's.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackKind {
    /// Immutable, digest-referenced rules.
    RulesDefinition,
    /// Mutable world state instantiated once.
    CampaignSeed,
    /// A region: frames, localities, connections, background (doc 14 §2.1).
    Region,
    /// A neighborhood: sites, structures, rooms, portals, features, NPCs (doc 14 §2.5).
    Neighborhood,
    /// A site added to a neighborhood after the fact — a hydration (doc 14 §4.5).
    Site,
}

impl PackKind {
    /// The word this kind renders as, and the word the hashed manifest carries.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PackKind::RulesDefinition => "rules_definition",
            PackKind::CampaignSeed => "campaign_seed",
            PackKind::Region => "region",
            PackKind::Neighborhood => "neighborhood",
            PackKind::Site => "site",
        }
    }
}

/// One `(pack, version)` dependency a manifest declares.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackRequirement {
    /// The pack depended on.
    pub id: CanonicalId,
    /// The exact version depended on — never a range.
    pub version: u32,
    /// The **content** the dependency must have, when the author pins it. It enters
    /// the hashed value only when it is `Some`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<DefinitionDigest>,
}

/// A pack's identity and provenance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(into = "PackManifestWire")]
pub struct PackManifest {
    id: CanonicalId,
    version: u32,
    kind: PackKind,
    digest: DefinitionDigest,
    requires: Vec<PackRequirement>,
}

/// The strict wire form of [`PackManifest`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackManifestWire {
    /// The pack id.
    pub id: CanonicalId,
    /// The pack version; `1` upwards.
    pub version: u32,
    /// What the pack is for.
    pub kind: PackKind,
    /// The declared digest, 64 lowercase hex digits — **not** part of what is hashed.
    pub digest: String,
    /// The exact `(pack, version)` pairs this pack needs.
    #[serde(default)]
    pub requires: Vec<PackRequirement>,
}

impl PackManifest {
    /// The one constructor, and therefore the one place the invariants live.
    ///
    /// # Errors
    ///
    /// [`PackError::ZeroVersion`], [`PackError::TooManyRequires`],
    /// [`PackError::SelfRequire`] and [`PackError::DuplicateRequire`].
    pub fn new(
        id: CanonicalId,
        version: u32,
        kind: PackKind,
        digest: DefinitionDigest,
        requires: Vec<PackRequirement>,
    ) -> Result<PackManifest, PackError> {
        if version == 0 {
            return Err(PackError::ZeroVersion);
        }
        if requires.len() > MAX_REQUIRES {
            return Err(PackError::TooManyRequires {
                actual: requires.len(),
                max: MAX_REQUIRES,
            });
        }
        let mut seen: BTreeSet<&CanonicalId> = BTreeSet::new();
        for requirement in &requires {
            if requirement.id == id {
                return Err(PackError::SelfRequire { id: id.to_string() });
            }
            if !seen.insert(&requirement.id) {
                return Err(PackError::DuplicateRequire {
                    id: requirement.id.to_string(),
                });
            }
        }
        Ok(PackManifest {
            id,
            version,
            kind,
            digest,
            requires,
        })
    }

    /// The pack id.
    #[must_use]
    pub fn id(&self) -> &CanonicalId {
        &self.id
    }

    /// The pack version.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// What the pack is for.
    #[must_use]
    pub fn kind(&self) -> PackKind {
        self.kind
    }

    /// The declared digest.
    #[must_use]
    pub fn digest(&self) -> DefinitionDigest {
        self.digest
    }

    /// The exact `(pack, version)` pairs this pack needs.
    #[must_use]
    pub fn requires(&self) -> &[PackRequirement] {
        &self.requires
    }

    /// The part of the manifest that is **hashed** — everything but `digest`.
    fn hashed_value(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id.as_str(),
            "version": self.version,
            "kind": self.kind.label(),
            "requires": self
                .requires
                .iter()
                .map(|r| match &r.digest {
                    Some(digest) => serde_json::json!({
                        "id": r.id.as_str(),
                        "version": r.version,
                        "digest": digest.to_hex(),
                    }),
                    None => serde_json::json!({ "id": r.id.as_str(), "version": r.version }),
                })
                .collect::<Vec<_>>(),
        })
    }
}

impl From<PackManifest> for PackManifestWire {
    fn from(value: PackManifest) -> PackManifestWire {
        PackManifestWire {
            id: value.id,
            version: value.version,
            kind: value.kind,
            digest: value.digest.to_hex(),
            requires: value.requires,
        }
    }
}

impl TryFrom<PackManifestWire> for PackManifest {
    type Error = PackError;

    fn try_from(value: PackManifestWire) -> Result<PackManifest, PackError> {
        let digest = DefinitionDigest::from_hex(&value.digest).map_err(|error| {
            PackError::BadDigestText {
                detail: error.to_string(),
            }
        })?;
        PackManifest::new(value.id, value.version, value.kind, digest, value.requires)
    }
}

impl<'de> Deserialize<'de> for PackManifest {
    fn deserialize<D>(deserializer: D) -> Result<PackManifest, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PackManifestWire::deserialize(deserializer)?;
        PackManifest::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// One entry, as authored: a kind and a definition the kind's owner decodes.
///
/// `def` is always a JSON object — a definition is a record, and every owner decodes
/// it as one — and an unknown top-level key beside `kind`/`def` is an error.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(into = "RawEntryWire")]
pub struct RawEntry {
    kind: CanonicalId,
    def: serde_json::Map<String, serde_json::Value>,
}

/// The strict wire form of [`RawEntry`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RawEntryWire {
    /// The entry kind, e.g. `room`, `site`, `npc`.
    pub kind: CanonicalId,
    /// The definition, decoded by the kind's owner.
    pub def: serde_json::Value,
}

impl RawEntry {
    /// The one constructor.
    ///
    /// # Errors
    ///
    /// [`PackError::EntryNotObject`] when `def` is not a JSON object.
    pub fn new(kind: CanonicalId, def: serde_json::Value) -> Result<RawEntry, PackError> {
        match def {
            serde_json::Value::Object(def) => Ok(RawEntry { kind, def }),
            _ => Err(PackError::EntryNotObject {
                kind: kind.to_string(),
            }),
        }
    }

    /// The entry kind.
    #[must_use]
    pub fn kind(&self) -> &CanonicalId {
        &self.kind
    }

    /// The definition, as authored.
    #[must_use]
    pub fn def(&self) -> &serde_json::Map<String, serde_json::Value> {
        &self.def
    }

    /// The definition as a `Value`, for decoding through serde.
    #[must_use]
    pub fn def_value(&self) -> serde_json::Value {
        serde_json::Value::Object(self.def.clone())
    }

    /// Decode the definition into an owner's typed record.
    ///
    /// # Errors
    ///
    /// [`PackError::EntryDecode`] naming the entry kind and serde's reason.
    pub fn decode<T: serde::de::DeserializeOwned>(&self) -> Result<T, PackError> {
        serde_json::from_value(self.def_value()).map_err(|error| PackError::EntryDecode {
            kind: self.kind.to_string(),
            detail: error.to_string(),
        })
    }

    /// The canonical value of this entry — what the entry digest covers.
    #[must_use]
    pub fn canonical_value(&self) -> serde_json::Value {
        serde_json::json!({ "kind": self.kind.as_str(), "def": self.def })
    }

    /// The entry's own digest: the revision the latch remembers, so an unchanged entry
    /// is recognized as unchanged whatever else in the pack moved.
    ///
    /// # Errors
    ///
    /// [`PackError::Oversized`] past [`MAX_ENTRY_BYTES`], and [`PackError::Encoding`].
    pub fn digest(&self) -> Result<DefinitionDigest, PackError> {
        digest_under(
            ENTRY_DIGEST_DOMAIN,
            &self.canonical_value(),
            MAX_ENTRY_BYTES,
        )
    }
}

impl From<RawEntry> for RawEntryWire {
    fn from(value: RawEntry) -> RawEntryWire {
        RawEntryWire {
            kind: value.kind,
            def: serde_json::Value::Object(value.def),
        }
    }
}

impl TryFrom<RawEntryWire> for RawEntry {
    type Error = PackError;

    fn try_from(value: RawEntryWire) -> Result<RawEntry, PackError> {
        RawEntry::new(value.kind, value.def)
    }
}

impl<'de> Deserialize<'de> for RawEntry {
    fn deserialize<D>(deserializer: D) -> Result<RawEntry, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = RawEntryWire::deserialize(deserializer)?;
        RawEntry::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// A loaded, digest-verified pack.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(into = "PackWire")]
pub struct Pack {
    manifest: PackManifest,
    entries: BTreeMap<CanonicalId, RawEntry>,
    brief: Option<serde_json::Value>,
}

/// The strict wire form of [`Pack`] — and the on-disk pack format.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackWire {
    /// Identity and provenance.
    pub manifest: PackManifestWire,
    /// The entries, keyed by their pack-local id; a repeated id is an error.
    #[serde(deserialize_with = "deserialize_unique_entries")]
    pub entries: BTreeMap<CanonicalId, RawEntry>,
    /// Optional authoring provenance: the prose brief and the answers that produced
    /// the pack (doc 14 §3.7). Kept verbatim, never interpreted here, and inside the
    /// digest like every other byte.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief: Option<serde_json::Value>,
}

fn deserialize_unique_entries<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<CanonicalId, RawEntry>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct Visitor;

    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<CanonicalId, RawEntry>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a table of pack entries keyed by their pack-local id")
        }

        fn visit_map<A>(self, mut map: A) -> Result<BTreeMap<CanonicalId, RawEntry>, A::Error>
        where
            A: serde::de::MapAccess<'de>,
        {
            let mut entries: BTreeMap<CanonicalId, RawEntry> = BTreeMap::new();
            // The key is read as text first so a refused id is named: a hyphenated
            // `room-a` fails as "entry room-a …", not as an unnamed byte.
            while let Some((raw_id, entry)) = map.next_entry::<String, RawEntry>()? {
                let id = CanonicalId::new(&raw_id).map_err(|error| {
                    serde::de::Error::custom(format!(
                        "entry {raw_id:?} is not a pack-local id ([a-z0-9_.], 64 bytes): {error}"
                    ))
                })?;
                if entries.contains_key(&id) {
                    return Err(serde::de::Error::custom(PackError::DuplicateEntry {
                        id: id.to_string(),
                    }));
                }
                if entries.len() >= MAX_PACK_ENTRIES {
                    return Err(serde::de::Error::custom(PackError::TooManyEntries {
                        actual: entries.len().saturating_add(1),
                        max: MAX_PACK_ENTRIES,
                    }));
                }
                entries.insert(id, entry);
            }
            Ok(entries)
        }
    }

    deserializer.deserialize_map(Visitor)
}

impl Pack {
    /// The one constructor. It does **not** verify the declared digest: [`load_pack`]
    /// does that, because verification is a property of the bytes a pack arrived as.
    ///
    /// # Errors
    ///
    /// [`PackError::TooManyEntries`] past [`MAX_PACK_ENTRIES`].
    pub fn new(
        manifest: PackManifest,
        entries: BTreeMap<CanonicalId, RawEntry>,
        brief: Option<serde_json::Value>,
    ) -> Result<Pack, PackError> {
        if entries.len() > MAX_PACK_ENTRIES {
            return Err(PackError::TooManyEntries {
                actual: entries.len(),
                max: MAX_PACK_ENTRIES,
            });
        }
        Ok(Pack {
            manifest,
            entries,
            brief,
        })
    }

    /// The authoring provenance carried with the pack, if any (doc 14 §3.7).
    #[must_use]
    pub fn brief(&self) -> Option<&serde_json::Value> {
        self.brief.as_ref()
    }

    /// Identity and provenance.
    #[must_use]
    pub fn manifest(&self) -> &PackManifest {
        &self.manifest
    }

    /// The entries, in id order — deterministic on every iteration.
    #[must_use]
    pub fn entries(&self) -> &BTreeMap<CanonicalId, RawEntry> {
        &self.entries
    }

    /// One entry.
    #[must_use]
    pub fn entry(&self, id: &CanonicalId) -> Option<&RawEntry> {
        self.entries.get(id)
    }

    /// The entries of one kind, in id order.
    pub fn entries_of<'a>(
        &'a self,
        kind: &'a CanonicalId,
    ) -> impl Iterator<Item = (&'a CanonicalId, &'a RawEntry)> + 'a {
        self.entries.iter().filter(move |(_, e)| e.kind() == kind)
    }

    /// Every kind the pack uses.
    #[must_use]
    pub fn kinds(&self) -> BTreeSet<CanonicalId> {
        self.entries.values().map(|e| e.kind().clone()).collect()
    }

    /// **Exactly the bytes the digest covers** — the manifest without its `digest`
    /// field, plus every entry as authored.
    #[must_use]
    pub fn canonical_value(&self) -> serde_json::Value {
        let entries: serde_json::Map<String, serde_json::Value> = self
            .entries
            .iter()
            .map(|(id, e)| (id.to_string(), e.canonical_value()))
            .collect();
        let mut value = serde_json::json!({
            "manifest": self.manifest.hashed_value(),
            "entries": entries,
        });
        if let (Some(brief), serde_json::Value::Object(map)) = (&self.brief, &mut value) {
            map.insert("brief".to_string(), brief.clone());
        }
        value
    }

    /// The digest this pack's content actually has.
    ///
    /// # Errors
    ///
    /// [`PackError::Encoding`] and [`PackError::Oversized`].
    pub fn computed_digest(&self) -> Result<DefinitionDigest, PackError> {
        pack_digest(&self.canonical_value())
    }
}

impl From<Pack> for PackWire {
    fn from(value: Pack) -> PackWire {
        PackWire {
            manifest: PackManifestWire::from(value.manifest),
            entries: value.entries,
            brief: value.brief,
        }
    }
}

impl TryFrom<PackWire> for Pack {
    type Error = PackError;

    fn try_from(value: PackWire) -> Result<Pack, PackError> {
        if value.entries.len() > MAX_PACK_ENTRIES {
            return Err(PackError::TooManyEntries {
                actual: value.entries.len(),
                max: MAX_PACK_ENTRIES,
            });
        }
        let manifest = PackManifest::try_from(value.manifest)?;
        Pack::new(manifest, value.entries, value.brief)
    }
}

impl<'de> Deserialize<'de> for Pack {
    fn deserialize<D>(deserializer: D) -> Result<Pack, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PackWire::deserialize(deserializer)?;
        Pack::try_from(wire).map_err(serde::de::Error::custom)
    }
}

fn digest_under(
    domain: &str,
    canonical: &serde_json::Value,
    max_bytes: usize,
) -> Result<DefinitionDigest, PackError> {
    DefinitionDigest::for_canonical_value_with_limit(domain, canonical, max_bytes).map_err(
        |error| {
            let detail = error.to_string();
            if detail.contains("limit") || detail.contains("byte") {
                PackError::Oversized {
                    max: max_bytes,
                    detail,
                }
            } else {
                PackError::Encoding { detail }
            }
        },
    )
}

/// **The one digest function in the crate.**
///
/// # Errors
///
/// [`PackError::Oversized`] past [`MAX_PACK_BYTES`], and [`PackError::Encoding`].
pub fn pack_digest(canonical: &serde_json::Value) -> Result<DefinitionDigest, PackError> {
    digest_under(PACK_DIGEST_DOMAIN, canonical, MAX_PACK_BYTES)
}

/// **Parse and verify one pack from its bytes.** No filesystem, no world, no clock.
///
/// Parse strictly, compute the digest over the canonical encoding, compare it with
/// what the manifest declared; a pack whose declared digest was computed over other
/// bytes fails **here**, naming both digests.
///
/// # Errors
///
/// [`PackError::Oversized`], [`PackError::Parse`] for anything the strict wire refuses
/// (an unknown key included), and [`PackError::DigestMismatch`].
pub fn load_pack(bytes: &str) -> Result<Pack, PackError> {
    let pack = parse_pack(bytes)?;
    let computed = pack.computed_digest()?;
    let declared = pack.manifest.digest();
    if computed != declared {
        return Err(PackError::DigestMismatch {
            declared: declared.to_hex(),
            computed: computed.to_hex(),
        });
    }
    Ok(pack)
}

/// Parse a pack **without** comparing its declared digest: the authoring half of
/// [`load_pack`], for computing the digest a new pack should declare. Everything else
/// is as strict.
///
/// # Errors
///
/// [`PackError::Oversized`] and [`PackError::Parse`].
pub fn parse_pack(bytes: &str) -> Result<Pack, PackError> {
    if bytes.len() > MAX_PACK_BYTES {
        return Err(PackError::Oversized {
            max: MAX_PACK_BYTES,
            detail: format!("{} bytes of source", bytes.len()),
        });
    }
    serde_json::from_str(bytes).map_err(|error| PackError::Parse {
        detail: error.to_string(),
    })
}

/// A loaded pack's identity, as [`check_requires`] needs it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadedPack {
    /// The pack's declared (and verified) digest.
    pub digest: DefinitionDigest,
}

/// Resolve a pack's `requires` against everything loaded, failing **closed**.
///
/// # Errors
///
/// [`PackError::MissingRequire`], [`PackError::RequireVersionMismatch`] and
/// [`PackError::RequireDigestMismatch`].
pub fn check_requires(
    pack: &Pack,
    loaded: &BTreeMap<(CanonicalId, u32), LoadedPack>,
) -> Result<(), PackError> {
    for requirement in pack.manifest.requires() {
        let key = (requirement.id.clone(), requirement.version);
        if let Some(dependency) = loaded.get(&key) {
            if let Some(declared) = requirement.digest
                && declared != dependency.digest
            {
                return Err(PackError::RequireDigestMismatch {
                    id: requirement.id.to_string(),
                    version: requirement.version,
                    declared: declared.to_hex(),
                    actual: dependency.digest.to_hex(),
                });
            }
            continue;
        }
        let present: Vec<String> = loaded
            .keys()
            .filter(|(id, _)| *id == requirement.id)
            .map(|(_, version)| version.to_string())
            .collect();
        if present.is_empty() {
            return Err(PackError::MissingRequire {
                id: requirement.id.to_string(),
                version: requirement.version,
            });
        }
        return Err(PackError::RequireVersionMismatch {
            id: requirement.id.to_string(),
            wanted: requirement.version,
            present: present.join(", "),
        });
    }
    Ok(())
}

/// The entry kinds the composed server can decode, declared by their owners at
/// composition. A pack naming any other kind is refused at plan time, naming the entry.
#[derive(Clone, Debug, Default)]
pub struct EntryKinds {
    kinds: BTreeMap<CanonicalId, &'static str>,
}

impl EntryKinds {
    /// An empty registry.
    #[must_use]
    pub fn new() -> EntryKinds {
        EntryKinds::default()
    }

    /// Declare that `owner` decodes `kind`. Redeclaring a kind under a different owner
    /// is refused: one kind, one decoder.
    ///
    /// # Errors
    ///
    /// [`PackError::KindOwnedElsewhere`].
    pub fn declare(&mut self, kind: CanonicalId, owner: &'static str) -> Result<(), PackError> {
        match self.kinds.get(&kind) {
            Some(existing) if *existing != owner => Err(PackError::KindOwnedElsewhere {
                kind: kind.to_string(),
                owner: (*existing).to_string(),
            }),
            _ => {
                self.kinds.insert(kind, owner);
                Ok(())
            }
        }
    }

    /// Who decodes `kind`, if anyone.
    #[must_use]
    pub fn owner_of(&self, kind: &CanonicalId) -> Option<&'static str> {
        self.kinds.get(kind).copied()
    }

    /// Every declared kind, in order.
    pub fn kinds(&self) -> impl Iterator<Item = &CanonicalId> {
        self.kinds.keys()
    }

    /// Refuse a pack that names a kind nobody decodes, naming the first such entry.
    ///
    /// # Errors
    ///
    /// [`PackError::UnknownEntryKind`].
    pub fn check(&self, pack: &Pack) -> Result<(), PackError> {
        for (id, entry) in pack.entries() {
            if !self.kinds.contains_key(entry.kind()) {
                return Err(PackError::UnknownEntryKind {
                    entry: id.to_string(),
                    kind: entry.kind().to_string(),
                });
            }
        }
        Ok(())
    }
}

/// Why a pack was refused.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PackError {
    /// The bytes are not a pack.
    #[error("not a pack: {detail}")]
    Parse {
        /// serde's reason.
        detail: String,
    },
    /// The declared digest is not 64 hex digits.
    #[error("the declared digest is unreadable: {detail}")]
    BadDigestText {
        /// The reason.
        detail: String,
    },
    /// The declared digest is not the content's.
    #[error(
        "the declared digest {declared} is not this pack's digest {computed}; the declaration was \
         computed over different or non-canonical bytes"
    )]
    DigestMismatch {
        /// What the manifest said.
        declared: String,
        /// What the bytes are.
        computed: String,
    },
    /// Version `0`.
    #[error("a pack version starts at one")]
    ZeroVersion,
    /// Past [`MAX_REQUIRES`].
    #[error("a pack declares {actual} dependencies; the maximum is {max}")]
    TooManyRequires {
        /// Declared.
        actual: usize,
        /// The cap.
        max: usize,
    },
    /// A pack that depends on itself.
    #[error("pack {id} declares itself as a dependency")]
    SelfRequire {
        /// The pack.
        id: String,
    },
    /// A dependency named twice.
    #[error("pack {id} is declared as a dependency more than once")]
    DuplicateRequire {
        /// The dependency.
        id: String,
    },
    /// A dependency this server does not load.
    #[error("requires {id}@{version}, which this server does not load")]
    MissingRequire {
        /// The dependency.
        id: String,
        /// The version wanted.
        version: u32,
    },
    /// A dependency loaded at another version.
    #[error("requires {id}@{wanted}, but this server loads {id}@{present}")]
    RequireVersionMismatch {
        /// The dependency.
        id: String,
        /// The version wanted.
        wanted: u32,
        /// The versions present.
        present: String,
    },
    /// A dependency loaded at another content.
    #[error(
        "requires {id}@{version} at digest {declared}, but this server loads that version at \
         {actual}; a pack is referenced by content, not by name"
    )]
    RequireDigestMismatch {
        /// The dependency.
        id: String,
        /// The version.
        version: u32,
        /// The digest wanted.
        declared: String,
        /// The digest present.
        actual: String,
    },
    /// An entry id used twice.
    #[error("entry {id} is declared more than once")]
    DuplicateEntry {
        /// The entry.
        id: String,
    },
    /// Past [`MAX_PACK_ENTRIES`].
    #[error("a pack declares {actual} entries; the maximum is {max}")]
    TooManyEntries {
        /// Declared.
        actual: usize,
        /// The cap.
        max: usize,
    },
    /// An entry whose `def` is not an object.
    #[error("an entry of kind {kind} has a definition that is not an object")]
    EntryNotObject {
        /// The kind.
        kind: String,
    },
    /// An entry the owner's record refused.
    #[error("an entry of kind {kind} does not decode: {detail}")]
    EntryDecode {
        /// The kind.
        kind: String,
        /// serde's reason.
        detail: String,
    },
    /// An entry kind nobody decodes.
    #[error("entry {entry} has kind {kind}, which nothing on this server decodes")]
    UnknownEntryKind {
        /// The entry.
        entry: String,
        /// The kind.
        kind: String,
    },
    /// A kind declared under two owners.
    #[error("entry kind {kind} is already decoded by {owner}")]
    KindOwnedElsewhere {
        /// The kind.
        kind: String,
        /// Who has it.
        owner: String,
    },
    /// Past a byte cap.
    #[error("the canonical encoding is past {max} bytes: {detail}")]
    Oversized {
        /// The cap.
        max: usize,
        /// The reason.
        detail: String,
    },
    /// The shared encoder refused for another reason.
    #[error("canonical encoding failed: {detail}")]
    Encoding {
        /// The reason.
        detail: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> CanonicalId {
        CanonicalId::new(s).expect("test id")
    }

    fn unsigned(id_str: &str, kind: &str, entries: &str, requires: &str) -> String {
        format!(
            r#"{{"manifest":{{"id":"{id_str}","version":1,"kind":"{kind}","digest":"{}","requires":{requires}}},"entries":{entries}}}"#,
            "0".repeat(64)
        )
    }

    fn signed(id_str: &str, kind: &str, entries: &str, requires: &str) -> String {
        let draft = unsigned(id_str, kind, entries, requires);
        let digest = parse_pack(&draft)
            .expect("draft parses")
            .computed_digest()
            .expect("draft digests");
        draft.replace(&"0".repeat(64), &digest.to_hex())
    }

    const ROOMS: &str = r#"{"room.a":{"kind":"room","def":{"name":"A"}},"room.b":{"kind":"room","def":{"name":"B","z":1}}}"#;

    #[test]
    fn a_signed_pack_loads_and_an_unsigned_one_names_both_digests() {
        let pack = load_pack(&signed("lot", "neighborhood", ROOMS, "[]")).expect("loads");
        assert_eq!(pack.entries().len(), 2);
        assert_eq!(pack.manifest().kind(), PackKind::Neighborhood);
        let err = load_pack(&unsigned("lot", "neighborhood", ROOMS, "[]")).expect_err("refused");
        assert!(matches!(err, PackError::DigestMismatch { .. }), "{err}");
    }

    #[test]
    fn identical_bytes_identical_digest_and_reordering_changes_nothing() {
        let a = signed("lot", "neighborhood", ROOMS, "[]");
        let reordered = a.replace(
            r#"{"kind":"room","def":{"name":"B","z":1}}"#,
            r#"{"def":{"z":1,"name":"B"},"kind":"room"}"#,
        );
        assert_ne!(a, reordered, "the bytes differ");
        let da = load_pack(&a).expect("a").computed_digest().expect("da");
        let db = load_pack(&reordered)
            .expect("reordered still loads")
            .computed_digest()
            .expect("db");
        assert_eq!(da, db, "key order is not content");
        let spaced = a.replace(r#""name":"B""#, r#""name":"B ""#);
        let err = load_pack(&spaced).expect_err("whitespace inside a value is content");
        assert!(matches!(err, PackError::DigestMismatch { .. }));
    }

    #[test]
    fn a_hyphenated_id_and_an_unknown_key_are_refused_at_parse() {
        let bad_id = ROOMS.replace("room.a", "room-a");
        let err = load_pack(&unsigned("lot", "neighborhood", &bad_id, "[]")).expect_err("refused");
        assert!(matches!(err, PackError::Parse { .. }), "{err}");
        assert!(
            err.to_string().contains("room-a") || err.to_string().contains("canonical"),
            "{err}"
        );
        let unknown = ROOMS.replace(r#""def":{"name":"A"}"#, r#""def":{"name":"A"},"extra":1"#);
        let err = load_pack(&unsigned("lot", "neighborhood", &unknown, "[]")).expect_err("refused");
        assert!(matches!(err, PackError::Parse { .. }), "{err}");
        let bad_kind =
            unsigned("lot", "neighborhood", ROOMS, "[]").replace("neighborhood", "borough");
        let err = load_pack(&bad_kind).expect_err("refused");
        assert!(matches!(err, PackError::Parse { .. }), "{err}");
    }

    #[test]
    fn a_duplicate_entry_and_a_non_object_def_are_refused() {
        let dup = r#"{"room.a":{"kind":"room","def":{}},"room.a":{"kind":"room","def":{}}}"#;
        let err = parse_pack(&unsigned("lot", "neighborhood", dup, "[]")).expect_err("dup");
        assert!(err.to_string().contains("more than once"), "{err}");
        let scalar = r#"{"room.a":{"kind":"room","def":3}}"#;
        let err = parse_pack(&unsigned("lot", "neighborhood", scalar, "[]")).expect_err("scalar");
        assert!(err.to_string().contains("not an object"), "{err}");
    }

    #[test]
    fn manifest_invariants_hold() {
        let d = DefinitionDigest::from_bytes([0; 32]);
        let req = |s: &str| PackRequirement {
            id: id(s),
            version: 1,
            digest: None,
        };
        assert!(matches!(
            PackManifest::new(id("a"), 0, PackKind::Region, d, vec![]),
            Err(PackError::ZeroVersion)
        ));
        assert!(matches!(
            PackManifest::new(id("a"), 1, PackKind::Region, d, vec![req("a")]),
            Err(PackError::SelfRequire { .. })
        ));
        assert!(matches!(
            PackManifest::new(id("a"), 1, PackKind::Region, d, vec![req("b"), req("b")]),
            Err(PackError::DuplicateRequire { .. })
        ));
        let many: Vec<PackRequirement> =
            (0..=MAX_REQUIRES).map(|i| req(&format!("p{i}"))).collect();
        assert!(matches!(
            PackManifest::new(id("a"), 1, PackKind::Region, d, many),
            Err(PackError::TooManyRequires { .. })
        ));
    }

    #[test]
    fn requires_fail_closed_by_version_and_by_content() {
        let region = load_pack(&signed("region.lot", "region", "{}", "[]")).expect("region");
        let requires = r#"[{"id":"region.lot","version":1}]"#;
        let hood = load_pack(&signed("lot", "neighborhood", ROOMS, requires)).expect("hood");
        let mut loaded = BTreeMap::new();
        assert!(matches!(
            check_requires(&hood, &loaded),
            Err(PackError::MissingRequire { .. })
        ));
        loaded.insert(
            (id("region.lot"), 2),
            LoadedPack {
                digest: region.manifest().digest(),
            },
        );
        assert!(matches!(
            check_requires(&hood, &loaded),
            Err(PackError::RequireVersionMismatch { .. })
        ));
        loaded.insert(
            (id("region.lot"), 1),
            LoadedPack {
                digest: region.manifest().digest(),
            },
        );
        check_requires(&hood, &loaded).expect("satisfied");
        let pinned = format!(
            r#"[{{"id":"region.lot","version":1,"digest":"{}"}}]"#,
            "1".repeat(64)
        );
        let hood = load_pack(&signed("lot", "neighborhood", ROOMS, &pinned)).expect("hood");
        assert!(matches!(
            check_requires(&hood, &loaded),
            Err(PackError::RequireDigestMismatch { .. })
        ));
    }

    #[test]
    fn entry_kinds_refuse_the_unknown_and_the_twice_owned() {
        let pack = load_pack(&signed("lot", "neighborhood", ROOMS, "[]")).expect("loads");
        let mut kinds = EntryKinds::new();
        let err = kinds.check(&pack).expect_err("nothing declared");
        assert!(matches!(err, PackError::UnknownEntryKind { ref entry, .. } if entry == "room.a"));
        kinds.declare(id("room"), "atlas").expect("declared");
        kinds.check(&pack).expect("known");
        assert!(matches!(
            kinds.declare(id("room"), "someone_else"),
            Err(PackError::KindOwnedElsewhere { .. })
        ));
        kinds
            .declare(id("room"), "atlas")
            .expect("idempotent for the same owner");
    }

    #[test]
    fn entry_digests_change_only_with_the_entry() {
        let a = load_pack(&signed("lot", "neighborhood", ROOMS, "[]")).expect("a");
        let edited = ROOMS.replace(r#""name":"A""#, r#""name":"A2""#);
        let b = load_pack(&signed("lot", "neighborhood", &edited, "[]")).expect("b");
        let ea = a.entry(&id("room.a")).expect("a").digest().expect("d");
        let eb = b.entry(&id("room.a")).expect("a").digest().expect("d");
        assert_ne!(ea, eb);
        let ba = a.entry(&id("room.b")).expect("b").digest().expect("d");
        let bb = b.entry(&id("room.b")).expect("b").digest().expect("d");
        assert_eq!(ba, bb, "an untouched entry keeps its revision");
        assert_ne!(a.manifest().digest(), b.manifest().digest());
    }

    #[test]
    fn the_brief_is_inside_the_digest_and_carried_verbatim() {
        let with = signed("lot", "neighborhood", ROOMS, "[]")
            .replace(r#""entries":"#, r#""brief":{"prose":"a lot"},"entries":"#);
        let err = load_pack(&with).expect_err("the brief moved the digest");
        assert!(matches!(err, PackError::DigestMismatch { .. }));
    }
}

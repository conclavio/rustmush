//! **The three persisted TacSim records** (10b §4.1's cut-1 row, category
//! *legacy-post-image*, future family `tacsim.pack`).
//!
//! Every one is a bounded whole-document catalog/workflow record with a strict
//! `TryFrom` wire re-running its constructor invariants and a deterministic key, and
//! **none takes part in authority or prepared mechanics** — that is exactly the narrow
//! exemption 08i answer 12 ratified, and the reason the `@tacsim` stanza in
//! `legacy-handlers.toml` may be `legacy-post-image` rather than `authoritative`.
//!
//! | record | key | what a corrupt one costs |
//! |---|---|---|
//! | [`PackRecord`] | `tacsim:pack:{id}@{version}` | the ledger row for a published version, **and its `digest` is what the boot gate compares** (independent verify P1): a loaded pack whose content disagrees with its row refuses the boot naming the file and both digests, so a published version is never re-signed in place. A corrupt row costs exactly that: the **key still counts**, so the version stays *published* and is never treated as new, but the digest it was first published under is gone with the body, so boot repairs the row from the loaded pack and **pages level 2** — for that one version the gate is disarmed until an operator re-checks it with `@tacsim/pack verify` |
//! | [`SeedApplication`] | `tacsim:seed:{pack}@{version}` | the once-per-`(pack, version)` latch. The **key still counts**: a corrupt latch still bars a re-seed, which is the only fail-**closed** direction available |
//! | [`PinRecord`] | `tacsim:pin:{subject}` | one subject's version pin. A lost pin does **not** float the subject onto a newer version — resolution refuses instead (§3.1 invariant (i)) |

use std::collections::BTreeMap;

use mush_core::{CanonicalId, ClockStamp, DefinitionDigest, ObjId};
use serde::{Deserialize, Serialize};

use crate::pack::{MAX_SEED_OBJECTS, PackError, PackKind};

/// The ledger row for one published `(pack, version)`.
///
/// It exists so *retirement* is detectable across a reboot: a version whose row is
/// stored but whose file this server no longer loads has been retired, and §3.1
/// invariant (i) says an unpinned consumer of it fails closed rather than silently
/// upgrading. Nothing else can tell the difference between "never published" and
/// "published and then withdrawn".
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(into = "PackRecordWire")]
pub struct PackRecord {
    id: CanonicalId,
    version: u32,
    kind: PackKind,
    digest: DefinitionDigest,
    entries: u32,
    first_seen: ClockStamp,
}

/// The strict wire form of [`PackRecord`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackRecordWire {
    /// The pack.
    pub id: CanonicalId,
    /// The version.
    pub version: u32,
    /// Rules or seed.
    pub kind: PackKind,
    /// The digest this version was published under, 64 lowercase hex digits.
    pub digest: String,
    /// How many entries it declared.
    pub entries: u32,
    /// When this server first saw it.
    pub first_seen: ClockStamp,
}

impl PackRecord {
    /// The one constructor.
    ///
    /// # Errors
    ///
    /// [`PackError::ZeroVersion`] and [`PackError::TooManyEntries`].
    pub fn new(
        id: CanonicalId,
        version: u32,
        kind: PackKind,
        digest: DefinitionDigest,
        entries: u32,
        first_seen: ClockStamp,
    ) -> Result<PackRecord, PackError> {
        if version == 0 {
            return Err(PackError::ZeroVersion);
        }
        let max = crate::pack::MAX_PACK_ENTRIES;
        if entries as usize > max {
            return Err(PackError::TooManyEntries {
                actual: entries as usize,
                max,
            });
        }
        Ok(PackRecord {
            id,
            version,
            kind,
            digest,
            entries,
            first_seen,
        })
    }

    /// The pack.
    #[must_use]
    pub fn id(&self) -> &CanonicalId {
        &self.id
    }

    /// The version.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// Rules or seed.
    #[must_use]
    pub fn kind(&self) -> PackKind {
        self.kind
    }

    /// The digest it was published under.
    #[must_use]
    pub fn digest(&self) -> DefinitionDigest {
        self.digest
    }

    /// How many entries it declared.
    #[must_use]
    pub fn entries(&self) -> u32 {
        self.entries
    }

    /// When this server first saw it.
    #[must_use]
    pub fn first_seen(&self) -> &ClockStamp {
        &self.first_seen
    }
}

impl From<PackRecord> for PackRecordWire {
    fn from(value: PackRecord) -> PackRecordWire {
        PackRecordWire {
            id: value.id,
            version: value.version,
            kind: value.kind,
            digest: value.digest.to_hex(),
            entries: value.entries,
            first_seen: value.first_seen,
        }
    }
}

impl TryFrom<PackRecordWire> for PackRecord {
    type Error = PackError;

    fn try_from(value: PackRecordWire) -> Result<PackRecord, PackError> {
        let digest = DefinitionDigest::from_hex(&value.digest).map_err(|error| {
            PackError::BadDigestText {
                detail: error.to_string(),
            }
        })?;
        PackRecord::new(
            value.id,
            value.version,
            value.kind,
            digest,
            value.entries,
            value.first_seen,
        )
    }
}

impl<'de> Deserialize<'de> for PackRecord {
    fn deserialize<D>(deserializer: D) -> Result<PackRecord, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PackRecordWire::deserialize(deserializer)?;
        PackRecord::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// **The seed latch** — §3.1 invariant (ii), written in the same `WriteBatch` as the
/// objects it names.
///
/// `placed` is a **deviation from 10b §3.1's field list, and it is load-bearing.** The
/// plan gives the record `objects: u32` alone; a count proves the seed ran but names
/// nothing it made, so after a reboot the corridor would be unreconstructible — no
/// route could be materialized, and "the three locations exist after `commit` and the
/// fixture is reproducible from an empty store" would be unanswerable. `objects` is
/// kept as the declared count and the wire refuses a record whose count and map
/// disagree, which turns the redundancy into an invariant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(into = "SeedApplicationWire")]
pub struct SeedApplication {
    pack: CanonicalId,
    version: u32,
    digest: DefinitionDigest,
    applied_at: ClockStamp,
    objects: u32,
    placed: BTreeMap<CanonicalId, ObjId>,
}

/// The strict wire form of [`SeedApplication`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedApplicationWire {
    /// The pack applied.
    pub pack: CanonicalId,
    /// The version applied.
    pub version: u32,
    /// The digest it was applied under, 64 lowercase hex digits.
    pub digest: String,
    /// The world-IC stamp at application.
    pub applied_at: ClockStamp,
    /// How many objects it created.
    pub objects: u32,
    /// Pack-local id to the object it created.
    pub placed: BTreeMap<CanonicalId, ObjId>,
}

impl SeedApplication {
    /// The one constructor.
    ///
    /// # Errors
    ///
    /// [`PackError::ZeroVersion`], [`PackError::TooManySeedObjects`] past
    /// [`MAX_SEED_OBJECTS`], and [`PackError::SeedCountMismatch`] when the declared
    /// count and the placement map disagree.
    pub fn new(
        pack: CanonicalId,
        version: u32,
        digest: DefinitionDigest,
        applied_at: ClockStamp,
        placed: BTreeMap<CanonicalId, ObjId>,
    ) -> Result<SeedApplication, PackError> {
        if version == 0 {
            return Err(PackError::ZeroVersion);
        }
        if placed.len() > MAX_SEED_OBJECTS {
            return Err(PackError::TooManySeedObjects {
                actual: placed.len(),
                max: MAX_SEED_OBJECTS,
            });
        }
        let objects = u32::try_from(placed.len()).map_err(|_| PackError::TooManySeedObjects {
            actual: placed.len(),
            max: MAX_SEED_OBJECTS,
        })?;
        Ok(SeedApplication {
            pack,
            version,
            digest,
            applied_at,
            objects,
            placed,
        })
    }

    /// The pack applied.
    #[must_use]
    pub fn pack(&self) -> &CanonicalId {
        &self.pack
    }

    /// The version applied.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// The digest it was applied under.
    #[must_use]
    pub fn digest(&self) -> DefinitionDigest {
        self.digest
    }

    /// The world-IC stamp at application.
    #[must_use]
    pub fn applied_at(&self) -> &ClockStamp {
        &self.applied_at
    }

    /// How many objects it created.
    #[must_use]
    pub fn objects(&self) -> u32 {
        self.objects
    }

    /// Pack-local id to the object it created.
    #[must_use]
    pub fn placed(&self) -> &BTreeMap<CanonicalId, ObjId> {
        &self.placed
    }
}

impl From<SeedApplication> for SeedApplicationWire {
    fn from(value: SeedApplication) -> SeedApplicationWire {
        SeedApplicationWire {
            pack: value.pack,
            version: value.version,
            digest: value.digest.to_hex(),
            applied_at: value.applied_at,
            objects: value.objects,
            placed: value.placed,
        }
    }
}

impl TryFrom<SeedApplicationWire> for SeedApplication {
    type Error = PackError;

    fn try_from(value: SeedApplicationWire) -> Result<SeedApplication, PackError> {
        // The cap is re-checked before anything is built, so a stored latch naming
        // a million objects is refused rather than materialized.
        if value.placed.len() > MAX_SEED_OBJECTS {
            return Err(PackError::TooManySeedObjects {
                actual: value.placed.len(),
                max: MAX_SEED_OBJECTS,
            });
        }
        if value.objects as usize != value.placed.len() {
            return Err(PackError::SeedCountMismatch {
                declared: value.objects,
                placed: value.placed.len(),
            });
        }
        let digest = DefinitionDigest::from_hex(&value.digest).map_err(|error| {
            PackError::BadDigestText {
                detail: error.to_string(),
            }
        })?;
        SeedApplication::new(
            value.pack,
            value.version,
            digest,
            value.applied_at,
            value.placed,
        )
    }
}

impl<'de> Deserialize<'de> for SeedApplication {
    fn deserialize<D>(deserializer: D) -> Result<SeedApplication, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = SeedApplicationWire::deserialize(deserializer)?;
        SeedApplication::try_from(wire).map_err(serde::de::Error::custom)
    }
}

/// One subject pinned to one exact `(pack, version)` (§3.1 invariant (i), D-E7-24).
///
/// A pin is what makes "versions coexist" safe: publishing `v2` cannot move a pinned
/// subject, and un-pinning is an explicit migration rather than a side effect of a
/// newer file appearing in `pack_paths`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(into = "PinRecordWire")]
pub struct PinRecord {
    subject: CanonicalId,
    pack: CanonicalId,
    version: u32,
    pinned_at: ClockStamp,
}

/// The strict wire form of [`PinRecord`].
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PinRecordWire {
    /// The pinned subject.
    pub subject: CanonicalId,
    /// The pack it is pinned to.
    pub pack: CanonicalId,
    /// The exact version it is pinned to.
    pub version: u32,
    /// When the pin was made.
    pub pinned_at: ClockStamp,
}

impl PinRecord {
    /// The one constructor.
    ///
    /// # Errors
    ///
    /// [`PackError::ZeroVersion`].
    pub fn new(
        subject: CanonicalId,
        pack: CanonicalId,
        version: u32,
        pinned_at: ClockStamp,
    ) -> Result<PinRecord, PackError> {
        if version == 0 {
            return Err(PackError::ZeroVersion);
        }
        Ok(PinRecord {
            subject,
            pack,
            version,
            pinned_at,
        })
    }

    /// The pinned subject.
    #[must_use]
    pub fn subject(&self) -> &CanonicalId {
        &self.subject
    }

    /// The pack it is pinned to.
    #[must_use]
    pub fn pack(&self) -> &CanonicalId {
        &self.pack
    }

    /// The exact version it is pinned to.
    #[must_use]
    pub fn version(&self) -> u32 {
        self.version
    }

    /// When the pin was made.
    #[must_use]
    pub fn pinned_at(&self) -> &ClockStamp {
        &self.pinned_at
    }
}

impl From<PinRecord> for PinRecordWire {
    fn from(value: PinRecord) -> PinRecordWire {
        PinRecordWire {
            subject: value.subject,
            pack: value.pack,
            version: value.version,
            pinned_at: value.pinned_at,
        }
    }
}

impl TryFrom<PinRecordWire> for PinRecord {
    type Error = PackError;

    fn try_from(value: PinRecordWire) -> Result<PinRecord, PackError> {
        PinRecord::new(value.subject, value.pack, value.version, value.pinned_at)
    }
}

impl<'de> Deserialize<'de> for PinRecord {
    fn deserialize<D>(deserializer: D) -> Result<PinRecord, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PinRecordWire::deserialize(deserializer)?;
        PinRecord::try_from(wire).map_err(serde::de::Error::custom)
    }
}

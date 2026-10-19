//! Permanent IC claim/close data. Decoding a cause never authorizes closing.

use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use serde::{Deserialize, Serialize};

use super::super::rating::RatingRef;
use super::super::records::{PublicationError, Record};
use super::super::{
    Capacity, CharterId, FiledRevisionId, LicenceId, LineId, RevisionIdentity, TermGeneration,
    TermPhase, TermWindow, WorldIcSeconds,
};
use super::{IcProtocol, Transition};

fn bad(_: impl std::fmt::Display) -> PublicationError {
    PublicationError::Invalid
}

/// The actual IC seal owner allocates this in its own durable namespace. It is
/// never accepted from an OOC match or used as proof that a sortie occurred.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub(crate) struct SortieNumber(u64);
impl TryFrom<u64> for SortieNumber {
    type Error = PublicationError;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            Err(PublicationError::Invalid)
        } else {
            Ok(Self(value))
        }
    }
}
impl From<SortieNumber> for u64 {
    fn from(value: SortieNumber) -> Self {
        value.0
    }
}
impl SortieNumber {
    pub(crate) const fn get(self) -> u64 {
        self.0
    }
}

fn world_stamp(stamp: &ClockStamp) -> Result<(), PublicationError> {
    if !matches!(stamp.clock, ClockRef::WorldIc { generation } if generation != 0) {
        return Err(PublicationError::Invalid);
    }
    Ok(())
}

/// Exact immutable Open evidence, constructed only by the closed owner writer.
/// Warrant is deliberately personal in this protocol; house dispatch is not
/// inferred from an object owner or a staff permission.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct OpenClaim {
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) protocol: IcProtocol,
    pub(crate) unit: ObjId,
    pub(crate) sortie: SortieNumber,
    pub(crate) dispatcher: ObjId,
    pub(crate) hull_title_revision: u64,
    pub(crate) licence: LicenceId,
    pub(crate) term: TermGeneration,
    pub(crate) charter: CharterId,
    pub(crate) line: LineId,
    pub(crate) filed: FiledRevisionId,
    pub(crate) revision: RevisionIdentity,
    pub(crate) rating: RatingRef,
    pub(crate) window: TermWindow,
    pub(crate) capacity: Capacity,
    pub(crate) at_ic: ClockStamp,
    /// Digest of the full actual Tacsim MindSeal, not a player receipt.
    pub(crate) seal_digest: DefinitionDigest,
    /// Canonical actual Tactics IC marker before/after identity.
    pub(crate) custody_digest: DefinitionDigest,
    /// Exact Sim IC exclusion row identity in the same owner union.
    pub(crate) exclusion_digest: DefinitionDigest,
}
impl OpenClaim {
    pub(crate) fn validate(&self) -> Result<(), PublicationError> {
        self.protocol.validate()?;
        world_stamp(&self.at_ic)?;
        if self.action != self.protocol.action(Transition::Open)?
            || self.unit.0 == 0
            || self.dispatcher.0 == 0
            || self.unit == self.dispatcher
            || self.hull_title_revision == 0
            || !matches!(
                self.window
                    .phase_at(WorldIcSeconds::from_stamp(&self.at_ic).map_err(bad)?),
                TermPhase::Active | TermPhase::Grace
            )
        {
            return Err(PublicationError::Invalid);
        }
        // This is the captured admission instant, never today's clock. Actual
        // admission independently rechecks the live term/clock through custody.
        Ok(())
    }
    pub(crate) fn digest(&self) -> Result<DefinitionDigest, PublicationError> {
        self.validate()?;
        DefinitionDigest::for_canonical_value(
            "roll.ic_mind_claim.open.v1",
            &serde_json::to_value(self).map_err(bad)?,
        )
        .map_err(bad)
    }
}

/// These are the only terminal physical events. Pilot exit, field powerdown,
/// engagement end, licence expiry, timeouts and host absence are excluded.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum CloseKind {
    ReturnedToBase { base: ObjId },
    PoweredDownInBase { base: ObjId },
    Destroyed {},
    RecoveredToBase { base: ObjId },
}
impl CloseKind {
    fn validate(&self, unit: ObjId) -> Result<(), PublicationError> {
        match self {
            Self::ReturnedToBase { base }
            | Self::PoweredDownInBase { base }
            | Self::RecoveredToBase { base }
                if base.0 == 0 || *base == unit =>
            {
                Err(PublicationError::Invalid)
            }
            _ => Ok(()),
        }
    }
}

/// Permanent cause paired to one actual opaque owner event. This serialized
/// record is history only and is never accepted as a request or owner output.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CloseCause {
    pub(crate) operation: CommitToken,
    pub(crate) action: DefinitionRef,
    pub(crate) protocol: IcProtocol,
    pub(crate) open: CommitToken,
    pub(crate) open_digest: DefinitionDigest,
    pub(crate) unit: ObjId,
    pub(crate) sortie: SortieNumber,
    /// Actual owner-allocated transition order. IC sortie numbers are the
    /// corresponding Open ordinals; this allows bounded historical capacity
    /// replay without ordering unrelated clock generations or commit hashes.
    pub(crate) transition: u64,
    pub(crate) event_sequence: u64,
    pub(crate) event_digest: DefinitionDigest,
    pub(crate) kind: CloseKind,
    pub(crate) at_ic: ClockStamp,
}
impl CloseCause {
    pub(crate) fn validate_for(&self, open: &OpenClaim) -> Result<(), PublicationError> {
        open.validate()?;
        self.protocol.validate()?;
        world_stamp(&self.at_ic)?;
        self.kind.validate(self.unit)?;
        if self.operation == open.operation
            || self.open != open.operation
            || self.open_digest != open.digest()?
            || self.unit != open.unit
            || self.sortie != open.sortie
            || self.transition <= open.sortie.get()
            || self.event_sequence == 0
            || (self.at_ic.clock == open.at_ic.clock && self.at_ic.at < open.at_ic.at)
            || self.action != self.protocol.action(Transition::Close)?
        {
            return Err(PublicationError::Mismatch);
        }
        // No current holder or same-clock requirement: actual stopped custody
        // drains even after an IC clock-generation change or actor deletion.
        Ok(())
    }
    pub(crate) fn same_event(&self, other: &Self) -> bool {
        self.open == other.open
            && self.open_digest == other.open_digest
            && self.unit == other.unit
            && self.sortie == other.sortie
            && self.event_sequence == other.event_sequence
            && self.event_digest == other.event_digest
            && self.kind == other.kind
    }
}

/// Precreated single history slot. Open facts are never rewritten by Close.
/// The core future-close reservation accounts for the maximum terminal image.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ClaimRecord {
    pub(crate) format: u16,
    pub(crate) open: OpenClaim,
    // Required null on Open; omission is not a closed or default state.
    #[serde(deserialize_with = "required_option")]
    pub(crate) closed: Option<CloseCause>,
}
fn required_option<'de, D: serde::Deserializer<'de>>(
    decoder: D,
) -> Result<Option<CloseCause>, D::Error> {
    Option::<CloseCause>::deserialize(decoder)
}
impl Record for ClaimRecord {
    const FAMILY: &'static str = "roll_mind_ic_claim";
    const MAX_BYTES: usize = 16384;
    fn key(&self) -> String {
        self.open.operation.to_hex()
    }
    fn validate(&self) -> Result<(), PublicationError> {
        if self.format != 1 {
            return Err(PublicationError::Invalid);
        }
        self.open.validate()?;
        if let Some(closed) = &self.closed {
            closed.validate_for(&self.open)?;
        }
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(bad)?,
            Self::MAX_BYTES,
        )
        .map_err(|_| PublicationError::Capacity)?;
        Ok(())
    }
}

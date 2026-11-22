//! Durable execution and transition order. These decoded records are history;
//! they cannot construct a StartPermit or CloseEvent.
use mush_core::participant::ParticipantError;
use mush_core::{ClockRef, ClockStamp, CommitToken, DefinitionDigest, DefinitionRef, ObjId};
use mush_roll::commerce::ic_claims::{IcClaimConfig, IcCloseKind};
use serde::{Deserialize, Serialize};

use super::{execution::Execution, protocol::MAX_RECORD_BYTES};
pub(super) const PREFIX: &str = "ic_sortie:";
pub(super) const HEAD: &str = "ic_sortie_head";
// Complete terminal metadata has its own strict bound; Open reserves this
// amount before custody. No dummy terminal event or padding is persisted.
pub(super) const MAX_CLOSE_BYTES: usize = 8192;
/// **The record's format** (doc 23 cut S.4b-1: 2 carries `Opening.transition`). Format 1 is
/// refused, never migrated: the book is pre-launch and holds no claim (doc 38's phase rule).
pub(super) const FORMAT: u16 = 2;
/// **Compacted closed record format** (P2-TSC-6): replaces heavy execution source/seal/unit
/// bodies on terminal non-coverage rows with compact cryptographic audit digests, reclaiming
/// `reserved_bytes` on close while preserving every four-owner boot invariant.
pub(super) const COMPACT_FORMAT: u16 = 3;
/// Maximum canonical JSON size of [`Head`] (including optional pruned checkpoint fields).
pub(super) const HEAD_MAX_BYTES: usize = 256;
pub(super) fn bad(reason: impl std::fmt::Display) -> ParticipantError {
    // The player's sentence stays generic; the reason is the operator's (doc 23 cut S.4b-2).
    tracing::debug!(target: "ic_claim", %reason, "tacsim: IC seal owner refused");
    ParticipantError::refused("IC retained execution or event identity differs")
}
pub(super) fn present<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CapturedProtocol {
    pub(super) manifest: DefinitionRef,
    pub(super) participants: [DefinitionRef; 4],
}
impl CapturedProtocol {
    pub(super) fn capture(config: &IcClaimConfig) -> Self {
        Self {
            manifest: config.manifest().clone(),
            participants: config.participants(),
        }
    }
    pub(super) fn config(&self) -> Result<IcClaimConfig, ParticipantError> {
        let seal = self.participants.get(1).ok_or_else(|| bad("seal"))?.clone();
        let config = IcClaimConfig::new(self.manifest.clone(), seal).map_err(bad)?;
        if config.participants() != self.participants {
            return Err(bad("participants"));
        }
        Ok(config)
    }
    fn action(&self, close: bool) -> Result<DefinitionRef, ParticipantError> {
        self.config()?
            .actions()
            .map_err(bad)?
            .get(usize::from(close))
            .cloned()
            .ok_or_else(|| bad("action"))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum EventKind {
    ReturnedToBase { base: ObjId },
    PoweredDownInBase { base: ObjId },
    Destroyed {},
    RecoveredToBase { base: ObjId },
}
impl EventKind {
    pub(super) fn data(&self) -> IcCloseKind {
        match *self {
            Self::ReturnedToBase { base } => IcCloseKind::ReturnedToBase { base },
            Self::PoweredDownInBase { base } => IcCloseKind::PoweredDownInBase { base },
            Self::Destroyed {} => IcCloseKind::Destroyed,
            Self::RecoveredToBase { base } => IcCloseKind::RecoveredToBase { base },
        }
    }
    fn validate(&self, unit: ObjId) -> Result<(), ParticipantError> {
        match *self {
            Self::ReturnedToBase { base }
            | Self::PoweredDownInBase { base }
            | Self::RecoveredToBase { base }
                if base.0 == 0 || base == unit =>
            {
                Err(bad("base"))
            }
            _ => Ok(()),
        }
    }
}
fn world(stamp: &ClockStamp) -> Result<(), ParticipantError> {
    if !matches!(stamp.clock,ClockRef::WorldIc{generation} if generation!=0) {
        return Err(bad("clock"));
    }
    Ok(())
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Opening {
    pub(super) request: DefinitionDigest,
    pub(super) operation: CommitToken,
    pub(super) action: DefinitionRef,
    pub(super) protocol: CapturedProtocol,
    pub(super) dispatcher: ObjId,
    /// **The book's transition ordinal of this Open** (doc 23 cut S.4b-1, round 2's 23-7): the
    /// book's own field, never read off the seal, so the seal's sortie can be the epoch's IC id
    /// (`SortieId::Ic`, D-23-14) while the Roll's `SortieNumber` keeps carrying the ordinal.
    pub(super) transition: u64,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub(super) withheld: Option<super::host::coverage::WithheldCapture>,
    pub(super) admission_digest: DefinitionDigest,
    pub(super) at_ic: ClockStamp,
    pub(super) unit: mush_tactics::TacticalUnit,
    pub(super) execution: Execution,
}
impl Opening {
    pub(super) fn validate(&self) -> Result<(), ParticipantError> {
        world(&self.at_ic)?;
        self.execution.validate()?;
        if let Some(withheld) = &self.withheld {
            withheld.validate(&self.unit, self.dispatcher)?;
        }
        if self.request.as_bytes() == &[0; 32]
            || self.action != self.protocol.action(false)?
            || self.transition == 0
            // The fixture's packed id is the ordinal itself (the shape before S.4b-1); an IC
            // epoch's id names the sealed hull.
            || match self.execution.seal.sortie {
                crate::arena::mind::SortieId::Packed(n) => n != self.transition,
                crate::arena::mind::SortieId::Ic { hull, .. } => hull != self.execution.seal.unit,
            }
            || self.dispatcher.0 == 0
            || self.unit.object() != self.execution.seal.unit
            || self.execution.seal.unit == self.dispatcher
            || self.execution.seal.principal
                != Some(crate::deliberation::order::CharacterRef::Character {
                    id: self.dispatcher,
                })
            || self.admission_digest.as_bytes() == &[0; 32]
        {
            return Err(bad("opening"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Closing {
    pub(super) request: DefinitionDigest,
    pub(super) operation: CommitToken,
    pub(super) action: DefinitionRef,
    pub(super) protocol: CapturedProtocol,
    pub(super) transition: u64,
    pub(super) sequence: u64,
    pub(super) event_digest: DefinitionDigest,
    pub(super) kind: EventKind,
    pub(super) at_ic: ClockStamp,
    pub(super) conditions_digest: DefinitionDigest,
    pub(super) unit_digest: DefinitionDigest,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "present"
    )]
    pub(super) coverage: Option<super::host::coverage::ClosedCoverageCapture>,
}
impl Closing {
    /// The actual host-event owner computes this after a genuine transition;
    /// the public protocol only reads its opaque prepared output.
    pub(super) fn event_digest(
        &self,
        open: &Opening,
    ) -> Result<DefinitionDigest, ParticipantError> {
        self.event_digest_parts(
            open.operation,
            open.execution.seal.unit,
            open.execution.seal.sortie,
            open.execution.digest(),
        )
    }
    pub(super) fn event_digest_parts(
        &self,
        open_operation: CommitToken,
        unit: ObjId,
        sortie: crate::arena::mind::SortieId,
        seal_digest: DefinitionDigest,
    ) -> Result<DefinitionDigest, ParticipantError> {
        let mut value = serde_json::json!({
            "request":self.request,"open":open_operation,"unit":unit,"sortie":sortie,
            "seal":seal_digest,"sequence":self.sequence,"kind":self.kind,
            "at_ic":self.at_ic,"conditions":self.conditions_digest,"unit_digest":self.unit_digest,
        });
        if let Some(coverage) = &self.coverage {
            value
                .as_object_mut()
                .ok_or_else(|| bad("close event"))?
                .insert(
                    "coverage".into(),
                    serde_json::to_value(coverage).map_err(bad)?,
                );
        }
        DefinitionDigest::for_canonical_value_with_limit(
            "tacsim.ic_host_event.v1",
            &value,
            MAX_CLOSE_BYTES,
        )
        .map_err(bad)
    }
    #[allow(clippy::too_many_arguments)]
    fn validate_common(
        &self,
        open_request: DefinitionDigest,
        open_operation: CommitToken,
        open_transition: u64,
        open_at_ic: &ClockStamp,
        unit: ObjId,
        sortie: crate::arena::mind::SortieId,
        seal_digest: DefinitionDigest,
    ) -> Result<(), ParticipantError> {
        world(&self.at_ic)?;
        self.kind.validate(unit)?;
        if self.request.as_bytes() == &[0; 32]
            || self.request == open_request
            || self.operation == open_operation
            || self.action != self.protocol.action(true)?
            || self.transition <= open_transition
            || self.sequence == 0
            || self.conditions_digest.as_bytes() == &[0; 32]
            || self.unit_digest.as_bytes() == &[0; 32]
            || self.event_digest
                != self.event_digest_parts(open_operation, unit, sortie, seal_digest)?
            || (self.at_ic.clock == open_at_ic.clock && self.at_ic.at < open_at_ic.at)
        {
            return Err(bad("closing"));
        }
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(bad)?,
            MAX_CLOSE_BYTES,
        )
        .map_err(bad)?;
        Ok(())
    }
    pub(super) fn validate(&self, open: &Opening) -> Result<(), ParticipantError> {
        if open.withheld.is_some() != self.coverage.is_some() {
            return Err(bad("close departure capture"));
        }
        if let Some(coverage) = &self.coverage {
            coverage.validate(open)?;
        }
        self.validate_common(
            open.request,
            open.operation,
            open.transition,
            &open.at_ic,
            open.execution.seal.unit,
            open.execution.seal.sortie,
            open.execution.digest(),
        )
    }
    pub(super) fn validate_compact(&self, open: &CompactOpen) -> Result<(), ParticipantError> {
        if self.coverage.is_some() {
            return Err(bad("compacted covered close"));
        }
        self.validate_common(
            open.request,
            open.operation,
            open.transition,
            &open.at_ic,
            open.unit,
            open.sortie,
            open.seal_digest,
        )
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub(super) format: u16,
    pub(super) open: Opening,
    #[serde(deserialize_with = "required_close")]
    pub(super) closed: Option<Closing>,
}
fn required_close<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Closing>, D::Error> {
    Option::deserialize(d)
}
impl Record {
    pub(super) fn key(&self) -> String {
        format!("{PREFIX}{}", self.open.operation.to_hex())
    }
    pub(super) fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != FORMAT {
            return Err(bad("format"));
        }
        self.open.validate()?;
        if let Some(c) = &self.closed {
            c.validate(&self.open)?;
        }
        self.reserved_bytes()?;
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(bad)?,
            MAX_RECORD_BYTES,
        )
        .map_err(bad)?;
        Ok(())
    }
    pub(super) fn reserved_bytes(&self) -> Result<usize, ParticipantError> {
        // The immutable Open body is never replaced. Null takes four bytes;
        // reserving the complete maximum terminal body is conservative by four.
        let open = serde_json::json!({"format":self.format,"open":self.open,"closed":null});
        let bytes = mush_core::canonical_json_bytes_with_limit(&open, MAX_RECORD_BYTES)
            .map_err(bad)?
            .len();
        let total = bytes
            .checked_add(MAX_CLOSE_BYTES)
            .ok_or_else(|| bad("capacity"))?;
        if total > MAX_RECORD_BYTES {
            return Err(bad("future close capacity"));
        }
        Ok(total)
    }
    pub(super) fn encoded_bytes(&self) -> Result<usize, ParticipantError> {
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(bad)?,
            MAX_RECORD_BYTES,
        )
        .map(|bytes| bytes.len())
        .map_err(bad)
    }
    pub(super) fn decode(value: &serde_json::Value) -> Result<Self, ParticipantError> {
        mush_core::canonical_json_bytes_with_limit(value, MAX_RECORD_BYTES).map_err(bad)?;
        let row: Self = serde_json::from_value(value.clone()).map_err(bad)?;
        row.validate()?;
        if serde_json::to_value(&row).map_err(bad)? != *value {
            return Err(bad("noncanonical record"));
        }
        Ok(row)
    }
}

/// Compact opening digest retained on closed non-coverage sorties after close compaction.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompactOpen {
    pub(super) request: DefinitionDigest,
    pub(super) operation: CommitToken,
    pub(super) action: DefinitionRef,
    pub(super) protocol: CapturedProtocol,
    pub(super) dispatcher: ObjId,
    pub(super) transition: u64,
    pub(super) admission_digest: DefinitionDigest,
    pub(super) at_ic: ClockStamp,
    pub(super) unit: ObjId,
    pub(super) sortie: crate::arena::mind::SortieId,
    pub(super) seal_digest: DefinitionDigest,
}
impl CompactOpen {
    pub(super) fn validate(&self) -> Result<(), ParticipantError> {
        world(&self.at_ic)?;
        if self.request.as_bytes() == &[0; 32]
            || self.action != self.protocol.action(false)?
            || self.transition == 0
            || match self.sortie {
                crate::arena::mind::SortieId::Packed(n) => n != self.transition,
                crate::arena::mind::SortieId::Ic { hull, .. } => hull != self.unit || hull.0 == 0,
            }
            || self.dispatcher.0 == 0
            || self.unit.0 == 0
            || self.unit == self.dispatcher
            || self.admission_digest.as_bytes() == &[0; 32]
            || self.seal_digest.as_bytes() == &[0; 32]
        {
            return Err(bad("compact opening"));
        }
        Ok(())
    }
}

/// Compact terminal record for closed non-coverage sorties (`format: 3`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CompactedRecord {
    pub(super) format: u16,
    pub(super) open: CompactOpen,
    pub(super) closed: Closing,
}
impl CompactedRecord {
    pub(super) fn from_closed(record: &Record) -> Result<Self, ParticipantError> {
        if record.open.withheld.is_some() {
            return Err(bad("covered record cannot compact"));
        }
        let closed = record
            .closed
            .as_ref()
            .ok_or_else(|| bad("open record cannot compact"))?
            .clone();
        let compact = Self {
            format: COMPACT_FORMAT,
            open: CompactOpen {
                request: record.open.request,
                operation: record.open.operation,
                action: record.open.action.clone(),
                protocol: record.open.protocol.clone(),
                dispatcher: record.open.dispatcher,
                transition: record.open.transition,
                admission_digest: record.open.admission_digest,
                at_ic: record.open.at_ic.clone(),
                unit: record.open.execution.seal.unit,
                sortie: record.open.execution.seal.sortie,
                seal_digest: record.open.execution.digest(),
            },
            closed,
        };
        compact.validate()?;
        Ok(compact)
    }
    pub(super) fn key(&self) -> String {
        format!("{PREFIX}{}", self.open.operation.to_hex())
    }
    pub(super) fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != COMPACT_FORMAT {
            return Err(bad("compact format"));
        }
        self.open.validate()?;
        self.closed.validate_compact(&self.open)?;
        self.encoded_bytes()?;
        Ok(())
    }
    pub(super) fn encoded_bytes(&self) -> Result<usize, ParticipantError> {
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::to_value(self).map_err(bad)?,
            MAX_RECORD_BYTES,
        )
        .map(|bytes| bytes.len())
        .map_err(bad)
    }
    pub(super) fn decode(value: &serde_json::Value) -> Result<Self, ParticipantError> {
        mush_core::canonical_json_bytes_with_limit(value, MAX_RECORD_BYTES).map_err(bad)?;
        let row: Self = serde_json::from_value(value.clone()).map_err(bad)?;
        row.validate()?;
        if serde_json::to_value(&row).map_err(bad)? != *value {
            return Err(bad("noncanonical compact record"));
        }
        Ok(row)
    }
}

/// Durable entry in [`super::state::IcSorties`]: full for open and covered sorties, compacted
/// for closed non-coverage sorties.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum StoredRecord {
    Full(Box<Record>),
    Compacted(Box<CompactedRecord>),
}
impl StoredRecord {
    pub(super) fn decode(value: &serde_json::Value) -> Result<Self, ParticipantError> {
        match value.get("format").and_then(serde_json::Value::as_u64) {
            Some(v) if v == u64::from(FORMAT) => {
                Record::decode(value).map(Box::new).map(Self::Full)
            }
            Some(v) if v == u64::from(COMPACT_FORMAT) => CompactedRecord::decode(value)
                .map(Box::new)
                .map(Self::Compacted),
            _ => Err(bad("format")),
        }
    }
    pub(super) fn to_value(&self) -> Result<serde_json::Value, ParticipantError> {
        match self {
            Self::Full(r) => serde_json::to_value(r).map_err(bad),
            Self::Compacted(c) => serde_json::to_value(c).map_err(bad),
        }
    }
    pub(super) fn key(&self) -> String {
        match self {
            Self::Full(r) => r.key(),
            Self::Compacted(c) => c.key(),
        }
    }
    pub(super) fn validate(&self) -> Result<(), ParticipantError> {
        match self {
            Self::Full(r) => r.validate(),
            Self::Compacted(c) => c.validate(),
        }
    }
    /// Conservative bytes reserved while open (`open + MAX_CLOSE_BYTES`), shrinking to actual
    /// encoded bytes once terminal so closing reclaims capacity immediately.
    pub(super) fn reserved_bytes(&self) -> Result<usize, ParticipantError> {
        match self {
            Self::Full(r) if r.closed.is_none() => r.reserved_bytes(),
            Self::Full(r) => r.encoded_bytes(),
            Self::Compacted(c) => c.encoded_bytes(),
        }
    }
    pub(super) fn open_operation(&self) -> CommitToken {
        match self {
            Self::Full(r) => r.open.operation,
            Self::Compacted(c) => c.open.operation,
        }
    }
    pub(super) fn open_request(&self) -> DefinitionDigest {
        match self {
            Self::Full(r) => r.open.request,
            Self::Compacted(c) => c.open.request,
        }
    }
    pub(super) fn open_transition(&self) -> u64 {
        match self {
            Self::Full(r) => r.open.transition,
            Self::Compacted(c) => c.open.transition,
        }
    }
    pub(super) fn open_dispatcher(&self) -> ObjId {
        match self {
            Self::Full(r) => r.open.dispatcher,
            Self::Compacted(c) => c.open.dispatcher,
        }
    }
    pub(super) fn open_at_ic(&self) -> &ClockStamp {
        match self {
            Self::Full(r) => &r.open.at_ic,
            Self::Compacted(c) => &c.open.at_ic,
        }
    }
    pub(super) fn open_action(&self) -> &DefinitionRef {
        match self {
            Self::Full(r) => &r.open.action,
            Self::Compacted(c) => &c.open.action,
        }
    }
    pub(super) fn unit(&self) -> ObjId {
        match self {
            Self::Full(r) => r.open.execution.seal.unit,
            Self::Compacted(c) => c.open.unit,
        }
    }
    pub(super) fn seal_digest(&self) -> DefinitionDigest {
        match self {
            Self::Full(r) => r.open.execution.digest(),
            Self::Compacted(c) => c.open.seal_digest,
        }
    }
    pub(super) fn closed(&self) -> Option<&Closing> {
        match self {
            Self::Full(r) => r.closed.as_ref(),
            Self::Compacted(c) => Some(&c.closed),
        }
    }
    pub(super) fn is_prunable(&self) -> bool {
        match self {
            Self::Full(r) => r.closed.is_some() && r.open.withheld.is_none(),
            Self::Compacted(_) => true,
        }
    }
    pub(super) fn as_full(&self) -> Option<&Record> {
        match self {
            Self::Full(r) => Some(r),
            Self::Compacted(_) => None,
        }
    }
}

/// Rolling SHA-256 checkpoint linking pruned terminal non-coverage sorties across restarts.
#[allow(clippy::too_many_arguments)]
pub(super) fn pruned_chain_digest(
    previous: Option<DefinitionDigest>,
    open_operation: CommitToken,
    close_operation: CommitToken,
    open_transition: u64,
    close_transition: u64,
    unit: ObjId,
    seal_digest: DefinitionDigest,
    close_unit_digest: DefinitionDigest,
) -> Result<DefinitionDigest, ParticipantError> {
    DefinitionDigest::for_canonical_value_with_limit(
        "tacsim.ic_sortie.pruned.v1",
        &serde_json::json!({
            "previous": previous,
            "open": open_operation,
            "close": close_operation,
            "open_transition": open_transition,
            "close_transition": close_transition,
            "unit": unit,
            "seal_digest": seal_digest,
            "close_unit_digest": close_unit_digest,
        }),
        512,
    )
    .map_err(bad)
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Head {
    pub(super) format: u16,
    pub(super) next_transition: u64,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub(super) pruned_count: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) pruned_digest: Option<DefinitionDigest>,
}
impl Head {
    pub(super) fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1
            || self.next_transition == 0
            || (self.pruned_count == 0) != self.pruned_digest.is_none()
            || self.pruned_digest.is_some_and(|d| d.as_bytes() == &[0; 32])
        {
            return Err(bad("ordinal head"));
        }
        // Account for the entire future width before the head is first stored.
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::json!({
                "format": 1,
                "next_transition": u64::MAX,
                "pruned_count": u64::MAX,
                "pruned_digest": DefinitionDigest::from_bytes([0xff; 32]),
            }),
            HEAD_MAX_BYTES,
        )
        .map_err(bad)?;
        Ok(())
    }
}

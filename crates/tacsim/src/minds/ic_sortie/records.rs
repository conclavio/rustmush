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
pub(super) fn bad(_: impl std::fmt::Display) -> ParticipantError {
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
        let mut value = serde_json::json!({
            "request":self.request,"open":open.operation,"unit":open.execution.seal.unit,"sortie":open.execution.seal.sortie.0,
            "seal":open.execution.digest(),"sequence":self.sequence,"kind":self.kind,
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
    pub(super) fn validate(&self, open: &Opening) -> Result<(), ParticipantError> {
        world(&self.at_ic)?;
        self.kind.validate(open.execution.seal.unit)?;
        if open.withheld.is_some() != self.coverage.is_some() {
            return Err(bad("close departure capture"));
        }
        if let Some(coverage) = &self.coverage {
            coverage.validate(open)?;
        }
        if self.request.as_bytes() == &[0; 32]
            || self.request == open.request
            || self.operation == open.operation
            || self.action != self.protocol.action(true)?
            || self.transition <= open.execution.seal.sortie.0
            || self.sequence == 0
            || self.conditions_digest.as_bytes() == &[0; 32]
            || self.unit_digest.as_bytes() == &[0; 32]
            || self.event_digest != self.event_digest(open)?
            || (self.at_ic.clock == open.at_ic.clock && self.at_ic.at < open.at_ic.at)
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
        if self.format != 1 {
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Head {
    pub(super) format: u16,
    pub(super) next_transition: u64,
}
impl Head {
    pub(super) fn validate(&self) -> Result<(), ParticipantError> {
        if self.format != 1 || self.next_transition == 0 {
            return Err(bad("ordinal head"));
        }
        // Account for the entire future width before the head is first stored.
        mush_core::canonical_json_bytes_with_limit(
            &serde_json::json!({"format":1,"next_transition":u64::MAX}),
            128,
        )
        .map_err(bad)?;
        Ok(())
    }
}

//! Passive ordered transcript observations for Core's two window room legs.
//! Decoding and hypothetical route input supply no admission or append authority.

use mush_core::{DefinitionDigest, ObjId, canonical_json_bytes_with_limit};
use serde::{Deserialize, Serialize};

use crate::ScenesState;
use crate::movement::{self, CaptureError, MovementCapture, PlannedSceneEvent};

pub const MAX_WINDOW_SCENE_CAPTURE_BYTES: usize = 64 * 1024;

/// Each original leg remains explicit, including a no-op or a loop back to
/// the source. Its events retain the legacy join-before-leave order.
#[derive(Debug, PartialEq, Serialize)]
pub struct WindowSceneCapture {
    format: u16,
    actor: ObjId,
    source: Option<ObjId>,
    admitted_room: ObjId,
    projected_room: ObjId,
    at: i64,
    legs: Vec<MovementCapture>,
}

impl WindowSceneCapture {
    pub const fn actor(&self) -> ObjId {
        self.actor
    }
    pub const fn source(&self) -> Option<ObjId> {
        self.source
    }
    pub const fn admitted_room(&self) -> ObjId {
        self.admitted_room
    }
    pub const fn projected_room(&self) -> ObjId {
        self.projected_room
    }
    pub const fn at(&self) -> i64 {
        self.at
    }
    pub fn legs(&self) -> &[MovementCapture] {
        &self.legs
    }
    pub fn events(&self) -> impl Iterator<Item = &PlannedSceneEvent> {
        self.legs.iter().flat_map(|leg| leg.events())
    }
    pub fn encode(&self) -> Result<Vec<u8>, CaptureError> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| CaptureError::Encoding)?;
        canonical_json_bytes_with_limit(&value, MAX_WINDOW_SCENE_CAPTURE_BYTES)
            .map_err(|_| CaptureError::Limit)
    }
    pub fn digest(&self) -> Result<DefinitionDigest, CaptureError> {
        let value = serde_json::from_slice(&self.encode()?).map_err(|_| CaptureError::Encoding)?;
        DefinitionDigest::for_canonical_value_with_limit(
            "scenes.window_movement.capture.v1",
            &value,
            MAX_WINDOW_SCENE_CAPTURE_BYTES,
        )
        .map_err(|_| CaptureError::Encoding)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, CaptureError> {
        if bytes.len() > MAX_WINDOW_SCENE_CAPTURE_BYTES {
            return Err(CaptureError::Limit);
        }
        let wire: Wire = serde_json::from_slice(bytes).map_err(|_| CaptureError::Invalid)?;
        if wire.legs.len() > 2 {
            return Err(CaptureError::Limit);
        }
        let mut legs = Vec::with_capacity(wire.legs.len());
        for leg in wire.legs {
            let bytes = canonical_json_bytes_with_limit(&leg, movement::MAX_MOVEMENT_CAPTURE_BYTES)
                .map_err(|_| CaptureError::Limit)?;
            legs.push(MovementCapture::decode(&bytes)?);
        }
        let capture = Self {
            format: wire.format,
            actor: wire.actor,
            source: wire.source,
            admitted_room: wire.admitted_room,
            projected_room: wire.projected_room,
            at: wire.at,
            legs,
        };
        if capture.encode()? != bytes {
            return Err(CaptureError::Invalid);
        }
        Ok(capture)
    }
    /// Re-observe the same original route/time against the real owner slot.
    /// Ledger tails and Core/Sim/Atlas identity still need independent binding.
    pub fn recheck(&self, state: &ScenesState) -> Result<(), CaptureError> {
        if capture(
            state,
            self.actor,
            self.source,
            self.admitted_room,
            self.projected_room,
            self.at,
        )? != *self
        {
            return Err(CaptureError::Invalid);
        }
        Ok(())
    }
    fn validate(&self) -> Result<(), CaptureError> {
        let count = if self.admitted_room == self.projected_room {
            1
        } else {
            2
        };
        if self.format != 1 || self.legs.len() != count {
            return Err(CaptureError::Invalid);
        }
        for (index, leg) in self.legs.iter().enumerate() {
            let (from, to) = if index == 0 {
                (self.source, self.admitted_room)
            } else {
                (Some(self.admitted_room), self.projected_room)
            };
            if leg.actor() != self.actor
                || leg.from() != from
                || leg.to() != to
                || leg.at() != self.at
            {
                return Err(CaptureError::Invalid);
            }
            let _ = leg.encode()?;
        }
        Ok(())
    }
}

/// Bound and capture each actual scene selection without touching transcripts.
pub fn capture(
    state: &ScenesState,
    actor: ObjId,
    source: Option<ObjId>,
    admitted_room: ObjId,
    projected_room: ObjId,
    at: i64,
) -> Result<WindowSceneCapture, CaptureError> {
    let mut legs = vec![movement::capture(state, actor, source, admitted_room, at)?];
    if projected_room != admitted_room {
        legs.push(movement::capture(
            state,
            actor,
            Some(admitted_room),
            projected_room,
            at,
        )?);
    }
    let capture = WindowSceneCapture {
        format: 1,
        actor,
        source,
        admitted_room,
        projected_room,
        at,
        legs,
    };
    let _ = capture.encode()?;
    Ok(capture)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Wire {
    format: u16,
    actor: ObjId,
    #[serde(deserialize_with = "required_source")]
    source: Option<ObjId>,
    admitted_room: ObjId,
    projected_room: ObjId,
    at: i64,
    legs: Vec<serde_json::Value>,
}
fn required_source<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<ObjId>, D::Error> {
    Option::<ObjId>::deserialize(d)
}

#[cfg(test)]
#[path = "movement_window_capture_tests.rs"]
mod tests;

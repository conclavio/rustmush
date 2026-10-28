//! Pure selection of scene transcript events for a physical actor move.
//!
//! A capture is bounded data, not an admission, receipt, or permission to append
//! history. The caller supplies the actual event-time route and UTC timestamp.

use mush_core::{ObjId, canonical_json_bytes_with_limit};
use serde::{Deserialize, Serialize};

use crate::{SceneEvent, SceneEventKind, SceneRecord, SceneStatus, SceneUid, ScenesState};

/// At most one destination entry and one source departure.
pub const MAX_MOVEMENT_EVENTS: usize = 2;
/// Capture refuses excessive membership before collecting any witnesses.
pub const MAX_MOVEMENT_WITNESSES: usize = 256;
/// Maximum canonical payload and decoder input length.
pub const MAX_MOVEMENT_CAPTURE_BYTES: usize = 32 * 1024;
/// Each capture performs at most two scans of this bounded scene map.
pub const MAX_MOVEMENT_CAPTURE_SCENES: usize = 4096;

/// One original scene event and the ledger to which it belongs.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlannedSceneEvent {
    scene: SceneUid,
    event: SceneEvent,
}

impl PlannedSceneEvent {
    #[must_use]
    pub const fn scene(&self) -> SceneUid {
        self.scene
    }

    #[must_use]
    pub const fn event(&self) -> &SceneEvent {
        &self.event
    }

    pub(crate) fn into_parts(self) -> (SceneUid, SceneEvent) {
        (self.scene, self.event)
    }
}

/// Immutable event-time facts. Decoding does not authenticate their origin.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MovementCapture {
    format: u16,
    actor: ObjId,
    from: Option<ObjId>,
    to: ObjId,
    at: i64,
    events: Vec<PlannedSceneEvent>,
}

impl MovementCapture {
    #[must_use]
    pub const fn actor(&self) -> ObjId {
        self.actor
    }

    #[must_use]
    pub const fn from(&self) -> Option<ObjId> {
        self.from
    }

    #[must_use]
    pub const fn to(&self) -> ObjId {
        self.to
    }

    #[must_use]
    pub const fn at(&self) -> i64 {
        self.at
    }

    #[must_use]
    pub fn events(&self) -> &[PlannedSceneEvent] {
        &self.events
    }

    /// Encode exact route, timestamps and original events as canonical JSON.
    pub fn encode(&self) -> Result<Vec<u8>, CaptureError> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| CaptureError::Encoding)?;
        canonical_json_bytes_with_limit(&value, MAX_MOVEMENT_CAPTURE_BYTES)
            .map_err(|_| CaptureError::Limit)
    }

    /// Decode only strict, bounded, canonical payloads. Missing nullable fields
    /// are refused; `from: null` is an explicit unlocated source.
    pub fn decode(bytes: &[u8]) -> Result<Self, CaptureError> {
        if bytes.len() > MAX_MOVEMENT_CAPTURE_BYTES {
            return Err(CaptureError::Limit);
        }
        let wire: CaptureWire = serde_json::from_slice(bytes).map_err(|_| CaptureError::Invalid)?;
        if wire.events.len() > MAX_MOVEMENT_EVENTS
            || wire
                .events
                .iter()
                .any(|row| row.event.witnesses.len() > MAX_MOVEMENT_WITNESSES)
        {
            return Err(CaptureError::Limit);
        }
        let capture = Self {
            format: wire.format,
            actor: wire.actor,
            from: wire.from,
            to: wire.to,
            at: wire.at,
            events: wire
                .events
                .into_iter()
                .map(|row| PlannedSceneEvent {
                    scene: row.scene,
                    event: SceneEvent {
                        kind: match row.event.kind {
                            MoveKind::Joined => SceneEventKind::Joined,
                            MoveKind::Left => SceneEventKind::Left,
                        },
                        actor: row.event.actor,
                        ic_time: row.event.ic_time,
                        at: row.event.at,
                        witnesses: row.event.witnesses,
                        detail: serde_json::json!({}),
                    },
                })
                .collect(),
        };
        if capture.encode()? != bytes {
            return Err(CaptureError::Invalid);
        }
        Ok(capture)
    }

    fn validate(&self) -> Result<(), CaptureError> {
        if self.events.len() > MAX_MOVEMENT_EVENTS
            || self
                .events
                .iter()
                .any(|row| row.event.witnesses.len() > MAX_MOVEMENT_WITNESSES)
        {
            return Err(CaptureError::Limit);
        }
        if self.format != 1 || (self.from == Some(self.to) && !self.events.is_empty()) {
            return Err(CaptureError::Invalid);
        }
        let mut joined = false;
        let mut left = false;
        for row in &self.events {
            if row.event.actor != Some(self.actor)
                || row.event.at != self.at
                || row
                    .event
                    .detail
                    .as_object()
                    .is_none_or(|detail| !detail.is_empty())
            {
                return Err(CaptureError::Invalid);
            }
            match row.event.kind {
                SceneEventKind::Joined if !joined && !left => joined = true,
                SceneEventKind::Left if !left && self.from.is_some() => left = true,
                _ => return Err(CaptureError::Invalid),
            }
        }
        if self.events.len() == 2 && self.events[0].scene == self.events[1].scene {
            return Err(CaptureError::Invalid);
        }
        Ok(())
    }
}

/// Capture refuses without returning a partial event set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureError {
    /// Scene, witness, event, or encoded-byte capacity was exceeded.
    Limit,
    /// The payload is not a strict canonical movement capture.
    Invalid,
    /// Serialization failed.
    Encoding,
}

impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Limit => "scene movement capture exceeds its bounds",
            Self::Invalid => "invalid scene movement capture",
            Self::Encoding => "scene movement capture encoding failed",
        })
    }
}

impl std::error::Error for CaptureError {}

/// Capture a movement against the real owner's borrowed state. Membership and
/// lookup bounds are checked before constructing any event or cloning members.
pub fn capture(
    state: &ScenesState,
    actor: ObjId,
    from: Option<ObjId>,
    to: ObjId,
    at: i64,
) -> Result<MovementCapture, CaptureError> {
    // A self-loop observes no scenes in the legacy path, including in a large
    // world; it remains an empty capture without a scene-map scan.
    if from != Some(to) && state.scenes.len() > MAX_MOVEMENT_CAPTURE_SCENES {
        return Err(CaptureError::Limit);
    }
    let selected = select(state, from, to);
    for (scene, _) in selected.iter().flatten() {
        let members = scene
            .participants
            .len()
            .checked_add(scene.observers.len())
            .ok_or(CaptureError::Limit)?;
        if members > MAX_MOVEMENT_WITNESSES {
            return Err(CaptureError::Limit);
        }
    }
    let capture = MovementCapture {
        format: 1,
        actor,
        from,
        to,
        at,
        events: materialize(selected, actor, at),
    };
    let _ = capture.encode()?;
    Ok(capture)
}

/// Existing ambient dispatch has no new capture limits. Sharing only selection
/// and event construction preserves its historical behavior and ordering.
pub(crate) fn legacy_events(
    state: &ScenesState,
    actor: ObjId,
    from: Option<ObjId>,
    to: ObjId,
    at: i64,
) -> Vec<PlannedSceneEvent> {
    materialize(select(state, from, to), actor, at)
}

fn select(
    state: &ScenesState,
    from: Option<ObjId>,
    to: ObjId,
) -> [Option<(&SceneRecord, SceneEventKind)>; 2] {
    if from == Some(to) {
        return [None, None];
    }
    // Preserve the existing lookup exactly: the first holding scene masks any
    // later scene at the same room, even when the first is paused.
    let active = |room| {
        state
            .active_scene_at(room)
            .filter(|scene| scene.status == SceneStatus::Active)
    };
    [
        active(to).map(|scene| (scene, SceneEventKind::Joined)),
        from.and_then(active)
            .map(|scene| (scene, SceneEventKind::Left)),
    ]
}

fn materialize(
    selected: [Option<(&SceneRecord, SceneEventKind)>; 2],
    actor: ObjId,
    at: i64,
) -> Vec<PlannedSceneEvent> {
    selected
        .into_iter()
        .flatten()
        .map(|(scene, kind)| PlannedSceneEvent {
            scene: scene.id,
            event: SceneEvent::new(scene, at, kind, Some(actor), serde_json::json!({})),
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CaptureWire {
    format: u16,
    actor: ObjId,
    #[serde(deserialize_with = "required_option")]
    from: Option<ObjId>,
    to: ObjId,
    at: i64,
    events: Vec<RowWire>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RowWire {
    scene: SceneUid,
    event: EventWire,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventWire {
    kind: MoveKind,
    #[serde(deserialize_with = "required_option")]
    actor: Option<ObjId>,
    ic_time: i64,
    at: i64,
    witnesses: Vec<ObjId>,
    // An empty, required object preserves the exact legacy event detail while
    // refusing arbitrary nested payloads or silently discarded fields.
    #[serde(rename = "detail")]
    _detail: EmptyDetail,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum MoveKind {
    Joined,
    Left,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyDetail {}

fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

#[cfg(test)]
#[path = "movement_tests.rs"]
mod tests;

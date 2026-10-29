//! Bounded discovery observations for the admitted and projected room legs.
//! These values supply no window attestation, write or movement authority.

use mush_core::{DefinitionDigest, ObjId, ObjKind, World, canonical_json_bytes_with_limit};
use serde::{Deserialize, Serialize};

use crate::module::AtlasState;
use crate::movement::{self, CaptureError, Discovery, MovementCapture, SlotPresence};
use crate::travel::KnownDoc;

pub const MAX_WINDOW_DISCOVERY_BYTES: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiscoveryLeg {
    #[serde(deserialize_with = "required")]
    from: Option<ObjId>,
    to: ObjId,
    discovery: Discovery,
}
impl DiscoveryLeg {
    pub const fn from(&self) -> Option<ObjId> {
        self.from
    }
    pub const fn to(&self) -> ObjId {
        self.to
    }
    pub const fn discovery(&self) -> &Discovery {
        &self.discovery
    }
    pub fn planned_known(&self) -> Option<KnownDoc> {
        movement::planned_discovery(&self.discovery)
    }
}

/// Passive ordered data. The second leg observes the first leg's proposed
/// knowledge postimage, preserving duplicate/FIFO behavior over a loop.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct WindowDiscoveryCapture {
    format: u16,
    actor: ObjId,
    source: Option<ObjId>,
    admitted_room: ObjId,
    projected_room: ObjId,
    at: i64,
    actor_kind: Option<ObjKind>,
    atlas_slot: SlotPresence,
    legs: Vec<DiscoveryLeg>,
}
impl WindowDiscoveryCapture {
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
    pub const fn actor_kind(&self) -> Option<ObjKind> {
        self.actor_kind
    }
    pub const fn atlas_slot(&self) -> SlotPresence {
        self.atlas_slot
    }
    pub fn legs(&self) -> &[DiscoveryLeg] {
        &self.legs
    }
    /// The final proposed document if any leg changes knowledge. No write
    /// capability is attached to the returned bounded document.
    pub fn planned_known(&self) -> Option<KnownDoc> {
        self.legs.iter().rev().find_map(DiscoveryLeg::planned_known)
    }
    pub fn encode(&self) -> Result<Vec<u8>, CaptureError> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| CaptureError::Encoding)?;
        canonical_json_bytes_with_limit(&value, MAX_WINDOW_DISCOVERY_BYTES)
            .map_err(|_| CaptureError::Limit)
    }
    pub fn decode(bytes: &[u8]) -> Result<Self, CaptureError> {
        if bytes.len() > MAX_WINDOW_DISCOVERY_BYTES {
            return Err(CaptureError::Limit);
        }
        let wire: Wire = serde_json::from_slice(bytes).map_err(|_| CaptureError::Invalid)?;
        if wire.legs.len() > 2 {
            return Err(CaptureError::Limit);
        }
        let capture = Self {
            format: wire.format,
            actor: wire.actor,
            source: wire.source,
            admitted_room: wire.admitted_room,
            projected_room: wire.projected_room,
            at: wire.at,
            actor_kind: wire.actor_kind,
            atlas_slot: wire.atlas_slot,
            legs: wire.legs,
        };
        if capture.encode()? != bytes {
            return Err(CaptureError::Invalid);
        }
        Ok(capture)
    }
    pub fn digest(&self) -> Result<DefinitionDigest, CaptureError> {
        let value = serde_json::from_slice(&self.encode()?).map_err(|_| CaptureError::Encoding)?;
        DefinitionDigest::for_canonical_value_with_limit(
            "atlas.window_discovery.capture.v1",
            &value,
            MAX_WINDOW_DISCOVERY_BYTES,
        )
        .map_err(|_| CaptureError::Encoding)
    }
    /// Compare the first leg's observations with another original capture.
    /// Equality of this data grants no admission or window selection authority.
    pub fn check_first(&self, first: &MovementCapture) -> Result<(), CaptureError> {
        if first.actor() != self.actor
            || first.from() != self.source
            || first.to() != self.admitted_room
            || first.at() != self.at
            || first.actor_kind() != self.actor_kind
            || first.atlas_slot() != self.atlas_slot
            || self
                .legs
                .first()
                .is_none_or(|leg| leg.discovery() != first.discovery())
        {
            return Err(CaptureError::Invalid);
        }
        Ok(())
    }
    pub fn recheck(&self, world: &World) -> Result<(), CaptureError> {
        if capture(
            world,
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
    /// Recheck Atlas-owned observations at the final owner bind. The genuine
    /// Core preparation separately retains actor kind and room preimages.
    pub(crate) fn recheck_owner(&self, state: &AtlasState) -> Result<(), CaptureError> {
        if capture_observed(
            Some(state),
            self.actor_kind,
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
            if leg.from != from || leg.to != to {
                return Err(CaptureError::Invalid);
            }
            movement::validate_discovery(&leg.discovery, self.actor_kind, self.atlas_slot)?;
        }
        if let (Some(first), Some(second)) = (self.legs.first(), self.legs.get(1))
            && let Discovery::Mapped { known_before, .. } = &first.discovery
            && let Discovery::Mapped {
                known_before: second_before,
                ..
            } = &second.discovery
        {
            let changed = first.planned_known();
            let expected = changed
                .as_ref()
                .map(|doc| doc.places.as_slice())
                .or(known_before.as_deref());
            if second_before.as_deref() != expected {
                return Err(CaptureError::Invalid);
            }
        }
        Ok(())
    }
}

/// Observe actual actor kind and the borrowed Atlas slot. The supplied route
/// remains hypothetical until the real Core/Sim/Atlas preparation joins it.
pub fn capture(
    world: &World,
    actor: ObjId,
    source: Option<ObjId>,
    admitted_room: ObjId,
    projected_room: ObjId,
    at: i64,
) -> Result<WindowDiscoveryCapture, CaptureError> {
    let state = world.module_state::<AtlasState>();
    let kind = world.get(actor).map(|object| object.kind);
    capture_observed(
        state,
        kind,
        actor,
        source,
        admitted_room,
        projected_room,
        at,
    )
}

fn capture_observed(
    state: Option<&AtlasState>,
    kind: Option<ObjKind>,
    actor: ObjId,
    source: Option<ObjId>,
    admitted_room: ObjId,
    projected_room: ObjId,
    at: i64,
) -> Result<WindowDiscoveryCapture, CaptureError> {
    let before = state
        .and_then(|state| state.travel.known.get(&actor))
        .map(Vec::as_slice);
    let first = movement::observe_discovery(state, kind, admitted_room, before)?;
    let changed = movement::planned_discovery(&first);
    let after = changed.as_ref().map(|doc| doc.places.as_slice()).or(before);
    let mut legs = vec![DiscoveryLeg {
        from: source,
        to: admitted_room,
        discovery: first,
    }];
    if projected_room != admitted_room {
        legs.push(DiscoveryLeg {
            from: Some(admitted_room),
            to: projected_room,
            discovery: movement::observe_discovery(state, kind, projected_room, after)?,
        });
    }
    let capture = WindowDiscoveryCapture {
        format: 1,
        actor,
        source,
        admitted_room,
        projected_room,
        at,
        actor_kind: kind,
        atlas_slot: if state.is_some() {
            SlotPresence::Present
        } else {
            SlotPresence::Absent
        },
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
    #[serde(deserialize_with = "required")]
    source: Option<ObjId>,
    admitted_room: ObjId,
    projected_room: ObjId,
    at: i64,
    #[serde(deserialize_with = "required")]
    actor_kind: Option<ObjKind>,
    atlas_slot: SlotPresence,
    legs: Vec<DiscoveryLeg>,
}
fn required<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}

#[cfg(test)]
#[path = "movement_window_discovery_tests.rs"]
mod tests;

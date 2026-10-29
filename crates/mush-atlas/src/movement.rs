//! Bounded observations of Atlas's existing movement selectors.
//!
//! This is data, not movement admission, a prepared write, or proof that an
//! observer has completed. Discovery can propose one known-places document;
//! selected window work still requires the actual Sim and Atlas owners. No
//! placement, portal, cell, callback, output, or persistence is executed here.

use mush_core::{ObjId, ObjKind, World, canonical_json_bytes_with_limit};
use mush_sim::{PlacementRecord, SpatialParent, SpatialState};
use serde::{Deserialize, Serialize};

use crate::module::AtlasState;
use crate::record::{MAX_KNOWN_PLACES, RecordKey};
use crate::travel::{KnownDoc, plan_learn};

/// Maximum canonical capture and decoder input length.
pub const MAX_MOVEMENT_CAPTURE_BYTES: usize = 128 * 1024;
/// Aggregate UTF-8 bytes in the selected neighborhood and prior known keys.
pub const MAX_MOVEMENT_KEY_BYTES: usize = 64 * 1024;
/// Maximum open-window rows inspected by an explicit capture.
pub const MAX_MOVEMENT_WINDOWS: usize = 128;
/// Maximum room memberships on one inspected window.
pub const MAX_MOVEMENT_WINDOW_ROOMS: usize = 4096;
/// Maximum aggregate room memberships across inspected windows.
pub const MAX_MOVEMENT_WINDOW_MEMBERSHIPS: usize = 65_536;

/// Absence is not an authenticated empty owner state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotPresence {
    Absent,
    Present,
}

/// The existing knowledge policy's result, without a write capability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum LearnChange {
    /// A repeated visit does not reorder or trim the existing vector.
    Unchanged,
    /// Append the selected neighborhood after dropping this many oldest keys.
    Append { evict_oldest: usize },
}

/// Only facts consulted by the legacy discovery observer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Discovery {
    NotPlayer,
    AtlasAbsent,
    Unmapped,
    Mapped {
        neighborhood: RecordKey,
        /// Missing document and an explicitly empty document remain distinct.
        #[serde(deserialize_with = "required_option")]
        known_before: Option<Vec<RecordKey>>,
        change: LearnChange,
    },
}

/// The part of Sim placement consulted by `pose_of`, without cloning a pose.
/// Interior and Attached mean no *direct* map pose, not no physical ancestry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SimPlacementObservation {
    SlotAbsent,
    NoRecord,
    Unplaced,
    Interior,
    Attached,
    Direct { domain: u64, revision: u64 },
}

impl SimPlacementObservation {
    fn direct_domain(self) -> Option<u64> {
        match self {
            Self::Direct { domain, .. } => Some(domain),
            _ => None,
        }
    }
}

/// A bounded scalar observation; geometry and composite payloads are not copied.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowProbe {
    key: u64,
    domain: u64,
    revision: u64,
    room_count: usize,
    covers_to: bool,
    drawn: bool,
}

impl WindowProbe {
    #[must_use]
    pub const fn key(&self) -> u64 {
        self.key
    }
    #[must_use]
    pub const fn domain(&self) -> u64 {
        self.domain
    }
    #[must_use]
    pub const fn revision(&self) -> u64 {
        self.revision
    }
    #[must_use]
    pub const fn room_count(&self) -> usize {
        self.room_count
    }
    #[must_use]
    pub const fn covers_to(&self) -> bool {
        self.covers_to
    }
    #[must_use]
    pub const fn drawn(&self) -> bool {
        self.drawn
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WindowSelection {
    AtlasAbsent,
    Present {
        /// Exact BTreeMap iteration order, including negative membership tests.
        windows: Vec<WindowProbe>,
        /// First covering entry, filtered for a composite only after selection.
        #[serde(deserialize_with = "required_option")]
        covering: Option<u64>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WindowObservation {
    NotActor,
    Observed {
        placement: SimPlacementObservation,
        selection: WindowSelection,
    },
}

/// What the old selector would attempt, never whether those attempts succeed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowEffect {
    NotApplicable,
    /// No direct map release or drawn-map placement was selected. Missing owner
    /// slots remain visible in the observations; this is not completed custody.
    NoSelectedEffect,
    NeedsOwnerComposition {
        release: Option<u64>,
        place_or_repose: Option<u64>,
    },
}

/// Immutable observations. Strict decoding does not authenticate their origin.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MovementCapture {
    format: u16,
    actor: ObjId,
    from: Option<ObjId>,
    to: ObjId,
    at: i64,
    actor_kind: Option<ObjKind>,
    atlas_slot: SlotPresence,
    discovery: Discovery,
    window: WindowObservation,
}

impl MovementCapture {
    /// Recheck only Atlas-owned facts at bind. Core retains the actor/room
    /// object images and the joint Sim participant retains placement facts.
    pub(crate) fn recheck_owner(&self, state: &AtlasState) -> Result<(), CaptureError> {
        let placement = match &self.window {
            WindowObservation::Observed { placement, .. } => *placement,
            WindowObservation::NotActor => SimPlacementObservation::SlotAbsent,
        };
        if capture_observed(
            Some(state),
            self.actor_kind,
            placement,
            self.actor,
            self.from,
            self.to,
            self.at,
        )? != *self
        {
            return Err(CaptureError::Invalid);
        }
        Ok(())
    }
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
    pub const fn actor_kind(&self) -> Option<ObjKind> {
        self.actor_kind
    }
    #[must_use]
    pub const fn atlas_slot(&self) -> SlotPresence {
        self.atlas_slot
    }
    #[must_use]
    pub const fn discovery(&self) -> &Discovery {
        &self.discovery
    }
    #[must_use]
    pub const fn window(&self) -> &WindowObservation {
        &self.window
    }

    /// The same known-places postimage the legacy learner would write, if new.
    /// This value grants no authority to install or persist it.
    #[must_use]
    pub fn planned_known(&self) -> Option<KnownDoc> {
        match &self.discovery {
            Discovery::Mapped {
                neighborhood,
                known_before,
                ..
            } => plan_learn(known_before.as_deref().unwrap_or_default(), neighborhood),
            _ => None,
        }
    }

    #[must_use]
    pub fn window_effect(&self) -> WindowEffect {
        let WindowObservation::Observed {
            placement,
            selection,
        } = &self.window
        else {
            return WindowEffect::NotApplicable;
        };
        let old = placement.direct_domain();
        let target = match selection {
            WindowSelection::AtlasAbsent => None,
            WindowSelection::Present { covering, .. } => *covering,
        };
        let release = old.filter(|domain| Some(*domain) != target);
        if release.is_none() && target.is_none() {
            WindowEffect::NoSelectedEffect
        } else {
            WindowEffect::NeedsOwnerComposition {
                release,
                place_or_repose: target,
            }
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, CaptureError> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| CaptureError::Encoding)?;
        canonical_json_bytes_with_limit(&value, MAX_MOVEMENT_CAPTURE_BYTES)
            .map_err(|_| CaptureError::Limit)
    }

    /// Strict canonical decoding; nullable fields must be explicitly present.
    pub fn decode(bytes: &[u8]) -> Result<Self, CaptureError> {
        if bytes.len() > MAX_MOVEMENT_CAPTURE_BYTES {
            return Err(CaptureError::Limit);
        }
        let wire: CaptureWire = serde_json::from_slice(bytes).map_err(|_| CaptureError::Invalid)?;
        let capture = Self {
            format: wire.format,
            actor: wire.actor,
            from: wire.from,
            to: wire.to,
            at: wire.at,
            actor_kind: wire.actor_kind,
            atlas_slot: wire.atlas_slot,
            discovery: wire.discovery,
            window: wire.window,
        };
        if capture.encode()? != bytes {
            return Err(CaptureError::Invalid);
        }
        Ok(capture)
    }

    fn validate(&self) -> Result<(), CaptureError> {
        if self.format != 1 {
            return Err(CaptureError::Invalid);
        }
        match &self.discovery {
            Discovery::NotPlayer if self.actor_kind != Some(ObjKind::Player) => {}
            Discovery::AtlasAbsent
                if self.actor_kind == Some(ObjKind::Player)
                    && self.atlas_slot == SlotPresence::Absent => {}
            Discovery::Unmapped
                if self.actor_kind == Some(ObjKind::Player)
                    && self.atlas_slot == SlotPresence::Present => {}
            Discovery::Mapped {
                neighborhood,
                known_before,
                change,
            } if self.actor_kind == Some(ObjKind::Player)
                && self.atlas_slot == SlotPresence::Present =>
            {
                check_knowledge(neighborhood, known_before.as_deref())?;
                if *change
                    != learn_change(known_before.as_deref().unwrap_or_default(), neighborhood)
                {
                    return Err(CaptureError::Invalid);
                }
            }
            _ => return Err(CaptureError::Invalid),
        }
        let actor = self.actor_kind.is_some_and(ObjKind::is_actor);
        match &self.window {
            WindowObservation::NotActor if !actor => {}
            WindowObservation::Observed { selection, .. } if actor => match selection {
                WindowSelection::AtlasAbsent if self.atlas_slot == SlotPresence::Absent => {}
                WindowSelection::Present { windows, covering }
                    if self.atlas_slot == SlotPresence::Present =>
                {
                    check_windows(windows.iter().map(|row| row.room_count), windows.len())?;
                    let mut previous = None;
                    for row in windows {
                        if previous.is_some_and(|key| key >= row.key)
                            || (row.covers_to && row.room_count == 0)
                        {
                            return Err(CaptureError::Invalid);
                        }
                        previous = Some(row.key);
                    }
                    let expected = windows
                        .iter()
                        .find(|row| row.covers_to)
                        .filter(|row| row.drawn)
                        .map(|row| row.domain);
                    if *covering != expected {
                        return Err(CaptureError::Invalid);
                    }
                }
                _ => return Err(CaptureError::Invalid),
            },
            _ => return Err(CaptureError::Invalid),
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CaptureError {
    #[error("Atlas movement capture exceeds its bounds")]
    Limit,
    #[error("invalid Atlas movement capture")]
    Invalid,
    #[error("Atlas movement capture encoding failed")]
    Encoding,
}

/// Observe the actual borrowed owner slots. `from` and `at` are original event
/// data supplied by the caller; this neither checks dispatch nor moves `actor`.
pub fn capture(
    world: &World,
    actor: ObjId,
    from: Option<ObjId>,
    to: ObjId,
    at: i64,
) -> Result<MovementCapture, CaptureError> {
    let kind = world.get(actor).map(|object| object.kind);
    let placement = if kind.is_some_and(ObjKind::is_actor) {
        match world.module_state::<SpatialState>() {
            Some(state) => observe_placement(state.placements.get(&actor)),
            None => SimPlacementObservation::SlotAbsent,
        }
    } else {
        SimPlacementObservation::SlotAbsent // unused by the skipped observer
    };
    capture_observed(
        world.module_state::<AtlasState>(),
        kind,
        placement,
        actor,
        from,
        to,
        at,
    )
}

fn capture_observed(
    state: Option<&AtlasState>,
    actor_kind: Option<ObjKind>,
    placement: SimPlacementObservation,
    actor: ObjId,
    from: Option<ObjId>,
    to: ObjId,
    at: i64,
) -> Result<MovementCapture, CaptureError> {
    let discovery = if actor_kind != Some(ObjKind::Player) {
        Discovery::NotPlayer
    } else if let Some(state) = state {
        if let Some(geo) = state.records.roomgeo.get(&to) {
            let before = state.travel.known.get(&actor);
            check_knowledge(&geo.neighborhood, before.map(Vec::as_slice))?;
            Discovery::Mapped {
                neighborhood: geo.neighborhood.clone(),
                change: learn_change(
                    before.map(Vec::as_slice).unwrap_or_default(),
                    &geo.neighborhood,
                ),
                known_before: before.cloned(),
            }
        } else {
            Discovery::Unmapped
        }
    } else {
        Discovery::AtlasAbsent
    };
    let window = if actor_kind.is_some_and(ObjKind::is_actor) {
        let selection = if let Some(state) = state {
            check_windows(
                state.windows.open.values().map(|entry| entry.rooms.len()),
                state.windows.open.len(),
            )?;
            WindowSelection::Present {
                covering: state
                    .windows
                    .drawn_covering_domain(to)
                    .map(|domain| domain.0),
                windows: state
                    .windows
                    .open
                    .iter()
                    .map(|(key, entry)| WindowProbe {
                        key: *key,
                        domain: entry.domain,
                        revision: entry.revision,
                        room_count: entry.rooms.len(),
                        covers_to: entry.rooms.contains(&to),
                        drawn: entry.composite.is_some(),
                    })
                    .collect(),
            }
        } else {
            WindowSelection::AtlasAbsent
        };
        WindowObservation::Observed {
            placement,
            selection,
        }
    } else {
        WindowObservation::NotActor
    };
    let capture = MovementCapture {
        format: 1,
        actor,
        from,
        to,
        at,
        actor_kind,
        atlas_slot: if state.is_some() {
            SlotPresence::Present
        } else {
            SlotPresence::Absent
        },
        discovery,
        window,
    };
    let _ = capture.encode()?;
    Ok(capture)
}

fn observe_placement(record: Option<&PlacementRecord>) -> SimPlacementObservation {
    let Some(record) = record else {
        return SimPlacementObservation::NoRecord;
    };
    match &record.parent {
        SpatialParent::Direct { domain, .. } => SimPlacementObservation::Direct {
            domain: domain.0,
            revision: record.revision,
        },
        SpatialParent::Unplaced => SimPlacementObservation::Unplaced,
        SpatialParent::Interior { .. } => SimPlacementObservation::Interior,
        SpatialParent::Attached { .. } => SimPlacementObservation::Attached,
    }
}

fn learn_change(known: &[RecordKey], place: &RecordKey) -> LearnChange {
    if known.contains(place) {
        LearnChange::Unchanged
    } else {
        LearnChange::Append {
            evict_oldest: known
                .len()
                .saturating_add(1)
                .saturating_sub(MAX_KNOWN_PLACES),
        }
    }
}

fn check_knowledge(place: &RecordKey, known: Option<&[RecordKey]>) -> Result<(), CaptureError> {
    let known = known.unwrap_or_default();
    if known.len() > MAX_KNOWN_PLACES {
        return Err(CaptureError::Limit);
    }
    let mut bytes = 0_usize;
    for key in std::iter::once(place).chain(known) {
        bytes = bytes
            .checked_add(key.pack.as_str().len())
            .and_then(|size| size.checked_add(key.entry.as_str().len()))
            .filter(|size| *size <= MAX_MOVEMENT_KEY_BYTES)
            .ok_or(CaptureError::Limit)?;
    }
    Ok(())
}

fn check_windows(counts: impl Iterator<Item = usize>, len: usize) -> Result<(), CaptureError> {
    if len > MAX_MOVEMENT_WINDOWS {
        return Err(CaptureError::Limit);
    }
    let mut total = 0_usize;
    for count in counts {
        if count > MAX_MOVEMENT_WINDOW_ROOMS {
            return Err(CaptureError::Limit);
        }
        total = total
            .checked_add(count)
            .filter(|count| *count <= MAX_MOVEMENT_WINDOW_MEMBERSHIPS)
            .ok_or(CaptureError::Limit)?;
    }
    Ok(())
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
    #[serde(deserialize_with = "required_option")]
    actor_kind: Option<ObjKind>,
    atlas_slot: SlotPresence,
    discovery: Discovery,
    window: WindowObservation,
}

fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    deserializer: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

#[cfg(test)]
#[path = "movement_tests.rs"]
mod tests;

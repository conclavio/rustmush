use crate::arena::mind::SortieId;
use crate::detachment::activation::ActiveDetachment;
use crate::detachment::planning::DraftRef;
use crate::surface::excursion::OpenSortie;
use mush_core::{CanonicalId, ClockRef, ClockStamp, ObjId};
use mush_roll::commerce::ic_claims::IcCloseKind;
use mush_sim::{DomainId, Pose, SlotId};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Bounded work index for outstanding deployment acts, not a location mirror.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightIndex {
    pub schema: u16,
    pub units: BTreeSet<ObjId>,
}
impl Default for FlightIndex {
    fn default() -> Self {
        Self {
            schema: 1,
            units: BTreeSet::new(),
        }
    }
}
impl FlightIndex {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || self.units.len() > 1024 || self.units.iter().any(|u| u.0 == 0) {
            return Err("Invalid outstanding flight index.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationId {
    pub organizer: ObjId,
    pub draft: mush_tactics::detachment::planning::PlanId,
    pub activation: u64,
}
impl OperationId {
    pub fn of(roster: &ActiveDetachment) -> Self {
        Self {
            organizer: roster.organizer,
            draft: roster.draft,
            activation: roster.revision,
        }
    }
    pub fn reference(self) -> DraftRef {
        DraftRef {
            organizer: self.organizer,
            id: self.draft,
        }
    }
    pub fn key(self) -> Result<String, String> {
        crate::detachment::activation::detachment_id(self.reference())?;
        if self.activation == 0 {
            return Err("Invalid operation revision.".into());
        }
        Ok(format!(
            "{}-{}-{}",
            self.organizer.0, self.draft.0, self.activation
        ))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationRecord {
    pub schema: u16,
    pub id: OperationId,
    pub roster: ActiveDetachment,
    pub began: ClockStamp,
    pub ended: Option<ClockStamp>,
    /// References remain discoverable after units join later operations.
    pub flights: Vec<FlightKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<Box<mush_tactics::detachment::roster::RosterCheckpoint>>,
}
impl OperationRecord {
    pub fn validate(&self) -> Result<(), String> {
        self.roster.validate()?;
        self.id.key()?;
        if let Some(saved) = &self.command {
            mush_tactics::detachment::roster::DetachmentRoster::restore(saved)
                .map_err(|e| e.to_string())?;
            let expected = super::field::roster(self)?;
            if !saved.matches_membership(&expected) {
                return Err("Field command membership differs from accepted roster.".into());
            }
        }
        if self.schema != 1
            || self.id != OperationId::of(&self.roster)
            || !self.roster.active()
            || !ic(&self.began)
            || self
                .ended
                .as_ref()
                .is_some_and(|end| !same_clock_order(&self.began, end))
            || self.flights.len() > 256
            || self
                .flights
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                != self.flights.len()
            || self.flights.iter().any(|key| {
                key.key().is_err()
                    || !self
                        .roster
                        .plan
                        .slots
                        .iter()
                        .any(|s| s.unit == Some(key.unit))
            })
        {
            return Err("Invalid detachment operation.".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightKey {
    pub unit: ObjId,
    pub ordinal: u64,
}
impl FlightKey {
    pub fn key(self) -> Result<String, String> {
        if self.unit.0 == 0 || self.ordinal == 0 {
            return Err("Invalid flight identity.".into());
        }
        Ok(format!("{}-{}", self.unit.0, self.ordinal))
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnitHead {
    pub schema: u16,
    pub flight: FlightKey,
}
impl UnitHead {
    pub fn validate(&self) -> Result<(), String> {
        self.flight.key()?;
        if self.schema != 1 {
            return Err("Invalid unit flight head.".into());
        }
        Ok(())
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Departing,
    Field,
    Returning,
    Returned,
    Cancelled,
}
impl Phase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Departing => "departure pending",
            Self::Field => "on the surface",
            Self::Returning => "return pending",
            Self::Returned => "returned",
            Self::Cancelled => "cancelled before departure",
        }
    }
    pub fn open(self) -> bool {
        matches!(self, Self::Departing | Self::Field | Self::Returning)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightClose {
    pub at: ClockStamp,
    pub kind: IcCloseKind,
    pub operator_aboard: bool,
    pub ordered_by: Option<ObjId>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flight {
    pub schema: u16,
    pub key: FlightKey,
    pub operation: OperationId,
    pub revision: u64,
    pub slot: u16,
    pub launcher: ObjId,
    pub operator: ObjId,
    pub carrier: ObjId,
    pub site: CanonicalId,
    pub domain: DomainId,
    pub ramp: Pose,
    pub bay_room: ObjId,
    pub bay_slot: SlotId,
    pub requested: ClockStamp,
    pub due: ClockStamp,
    pub phase: Phase,
    /// Retained after closure: editing/disbanding a roster never erases a seal.
    pub epoch: Option<OpenSortie>,
    pub carrier_epoch: Option<SortieId>,
    pub closed: Option<FlightClose>,
    pub returner: Option<ObjId>,
}
fn ic(stamp: &ClockStamp) -> bool {
    matches!(stamp.clock, ClockRef::WorldIc { generation } if generation > 0)
}
fn same_clock_order(from: &ClockStamp, to: &ClockStamp) -> bool {
    matches!((&from.clock,&to.clock),(ClockRef::WorldIc{generation:a},ClockRef::WorldIc{generation:b}) if *a>0&&a<=b)
        && from.at <= to.at
}
pub(super) fn known_clock(world: &mush_core::World, stamp: &ClockStamp) -> bool {
    matches!(stamp.clock,ClockRef::WorldIc{generation} if world.world_timeline().epochs().iter().any(|epoch|epoch.generation==generation))
}
impl Flight {
    pub fn continuation_actor(&self) -> ObjId {
        if self.phase == Phase::Returning {
            self.returner.unwrap_or(self.launcher)
        } else {
            self.launcher
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        self.key.key()?;
        self.operation.key()?;
        let opened = self.epoch.as_ref();
        let shape = match self.phase {
            Phase::Departing | Phase::Cancelled => opened.is_none() && self.closed.is_none(),
            Phase::Field | Phase::Returning => opened.is_some() && self.closed.is_none(),
            Phase::Returned => opened.is_some() && self.closed.is_some(),
        };
        if self.schema != 1
            || self.revision == 0
            || !(1..=6).contains(&self.slot)
            || self.launcher != self.operation.organizer
            || self.operator.0 == 0
            || self.carrier.0 == 0
            || self.domain.0 == 0
            || self.bay_room.0 == 0
            || !same_clock_order(&self.requested, &self.due)
            || !shape
            || self.returner.is_some_and(|actor| actor.0 == 0)
            || self.phase == Phase::Returning && self.returner.is_none()
            || opened.is_some_and(|epoch| {
                epoch.id != SortieId::ic(self.key.unit, self.requested.at)
                    || epoch.hull != self.carrier
                    || epoch.operator != self.operator
                    || epoch.site != self.site
                    || u16::from(epoch.slot) != self.slot
                    || epoch.opened_ic < self.requested.at
            })
            || self.closed.as_ref().is_some_and(|close| {
                !same_clock_order(&self.requested, &close.at)
                    || opened.is_none_or(|epoch| close.at.at < epoch.opened_ic)
            })
        {
            return Err("Invalid unit flight.".into());
        }
        Ok(())
    }
}

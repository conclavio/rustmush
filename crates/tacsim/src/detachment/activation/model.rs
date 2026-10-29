use crate::detachment::crew::CrewDuty;
use crate::detachment::planning::DraftRef;
use mush_core::{CanonicalHash, ClockRef, ClockStamp, ObjId};
use mush_tactics::detachment::planning::{Plan, PlanHash, PlanId, PlannedController};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Exact compound identity, not a hash. First activation supports 32-bit local
/// draft IDs; larger IDs refuse rather than truncate or alias another roster.
pub fn detachment_id(reference: DraftRef) -> Result<u64, String> {
    let local =
        u32::try_from(reference.id.0).map_err(|_| "Draft ID exceeds activation capacity.")?;
    if reference.organizer.0 == 0 || local == 0 {
        return Err("Invalid detachment identity.".into());
    }
    Ok((u64::from(reference.organizer.0) << 32) | u64::from(local))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActiveDetachment {
    pub schema: u16,
    pub organizer: ObjId,
    pub draft: PlanId,
    pub detachment: u64,
    /// Advances on every new activation, independently of mutable draft edits.
    pub revision: u64,
    pub source_revision: u64,
    pub plan_hash: PlanHash,
    pub plan: Plan,
    pub duty: CrewDuty,
    pub npc_terms: BTreeMap<ObjId, CanonicalHash>,
    pub activated_at: ClockStamp,
    #[serde(deserialize_with = "required_option")]
    pub closed_at: Option<ClockStamp>,
}
fn required_option<'de, D: serde::Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(d)
}
fn ic(stamp: &ClockStamp) -> bool {
    matches!(stamp.clock, ClockRef::WorldIc { generation } if generation > 0)
}
impl ActiveDetachment {
    pub fn reference(&self) -> DraftRef {
        DraftRef {
            organizer: self.organizer,
            id: self.draft,
        }
    }
    pub fn active(&self) -> bool {
        self.closed_at.is_none()
    }
    pub fn entities(&self) -> BTreeSet<ObjId> {
        self.plan
            .slots
            .iter()
            .flat_map(|slot| slot.unit.into_iter().chain(slot.controller.character()))
            .collect()
    }
    pub fn validate(&self) -> Result<(), String> {
        self.plan.validate().map_err(|e| e.to_string())?;
        let npcs: BTreeSet<_> = self
            .plan
            .slots
            .iter()
            .filter_map(|s| match s.controller {
                PlannedController::Npc { character } => Some(character),
                _ => None,
            })
            .collect();
        if self.schema != 1
            || self.detachment != detachment_id(self.reference())?
            || self.revision == 0
            || self.source_revision == 0
            || self.plan.material_hash().map_err(|e| e.to_string())? != self.plan_hash
            || self.plan.slots.is_empty()
            || self.plan.financial_terms.is_some()
            || self.plan.slots.iter().any(|s| {
                s.unit.is_none()
                    || s.mind.is_some()
                    || !matches!(
                        s.controller,
                        PlannedController::Player { .. } | PlannedController::Npc { .. }
                    )
            })
            || self.entities().len() != self.plan.slots.len() * 2
            || self.entities().contains(&self.plan.carrier)
            || self.npc_terms.keys().copied().collect::<BTreeSet<_>>() != npcs
            || self
                .plan
                .npc_commander
                .is_some_and(|npc| !npcs.contains(&npc))
            || !ic(&self.activated_at)
            || self.closed_at.as_ref().is_some_and(|stamp| {
                !ic(stamp)
                    || (stamp.clock == self.activated_at.clock && stamp.at < self.activated_at.at)
            })
        {
            return Err("Invalid accepted detachment.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Reservation {
    pub schema: u16,
    pub entity: ObjId,
    pub organizer: ObjId,
    pub draft: PlanId,
    pub detachment: u64,
    pub revision: u64,
}
impl Reservation {
    pub fn for_entity(record: &ActiveDetachment, entity: ObjId) -> Self {
        Self {
            schema: 1,
            entity,
            organizer: record.organizer,
            draft: record.draft,
            detachment: record.detachment,
            revision: record.revision,
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || self.entity.0 == 0
            || self.revision == 0
            || self.detachment
                != detachment_id(DraftRef {
                    organizer: self.organizer,
                    id: self.draft,
                })?
        {
            return Err("Invalid detachment reservation.".into());
        }
        Ok(())
    }
}

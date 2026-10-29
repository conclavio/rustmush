//! Current personnel observations for preparation, not reservations or authority.
use super::{CrewContract, HireTerms, NpcMissionScope};
use mush_core::{Flags, ObjId, ObjKind, World};
use mush_tactics::detachment::Slot;
use mush_tactics::detachment::planning::{Draft, PlannedController};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PersonnelIssue {
    Unavailable,
    NotAboard,
    NotContracted,
    CommandOutsideContract,
    CommanderUnassigned,
}
impl PersonnelIssue {
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Unavailable => "character or contract unavailable",
            Self::NotAboard => "not currently aboard the selected carrier",
            Self::NotContracted => "no crew contract with this organizer",
            Self::CommandOutsideContract => "command is outside the agreed contract scope",
            Self::CommanderUnassigned => "commander must occupy an NPC roster slot",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NpcPersonnel {
    pub(crate) character: ObjId,
    pub(crate) slot: Option<Slot>,
    pub(crate) commander: bool,
    pub(crate) terms: Result<HireTerms, PersonnelIssue>,
}
fn check(
    world: &World,
    draft: &Draft,
    npc: ObjId,
    commander: bool,
    contract: Option<CrewContract>,
) -> Result<HireTerms, PersonnelIssue> {
    // A draft preference alone cannot query another person's existence or
    // whereabouts. Establish this organizer's employment relationship first.
    let contract = contract
        .filter(|contract| contract.employer == draft.organizer)
        .ok_or(PersonnelIssue::NotContracted)?;
    contract
        .validate()
        .map_err(|_| PersonnelIssue::Unavailable)?;
    if contract.npc != npc {
        return Err(PersonnelIssue::Unavailable);
    }
    if world
        .get(npc)
        .is_none_or(|o| o.kind != ObjKind::Npc || o.flags.contains(Flags::GOING))
    {
        return Err(PersonnelIssue::Unavailable);
    }
    if !crate::detachment::planning::projection::physically_aboard(world, npc, draft.plan.carrier) {
        return Err(PersonnelIssue::NotAboard);
    }
    if commander && contract.terms.mission_scope != NpcMissionScope::DetachmentCommander {
        return Err(PersonnelIssue::CommandOutsideContract);
    }
    Ok(contract.terms)
}
/// Reads the six-slot draft and current hull containment. Recompute at
/// activation/launch; none of these observations grants control or proves health.
pub(crate) fn resolve(
    world: &World,
    draft: &Draft,
    mut load: impl FnMut(ObjId) -> Result<Option<CrewContract>, String>,
) -> Result<Vec<NpcPersonnel>, String> {
    draft.validate().map_err(|e| e.to_string())?;
    let mut rows = Vec::new();
    for slot in &draft.plan.slots {
        let PlannedController::Npc { character } = slot.controller else {
            continue;
        };
        let commander = draft.plan.npc_commander == Some(character);
        let terms = match load(character) {
            Ok(contract) => check(world, draft, character, commander, contract),
            Err(_) => Err(PersonnelIssue::Unavailable),
        };
        rows.push(NpcPersonnel {
            character,
            slot: Some(slot.slot),
            commander,
            terms,
        });
    }
    if let Some(commander) = draft.plan.npc_commander
        && !rows.iter().any(|row| row.character == commander)
    {
        rows.push(NpcPersonnel {
            character: commander,
            slot: None,
            commander: true,
            terms: Err(PersonnelIssue::CommanderUnassigned),
        });
    }
    Ok(rows)
}

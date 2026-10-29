//! Port hiring records for real NPCs. Boarding remains ordinary movement.
mod commands;
pub(crate) mod personnel;
mod runtime;

pub(crate) use commands::describe as describe_terms;
pub(crate) use commands::{SWITCHES, command, handles};
use mush_core::{
    CanonicalHash, CanonicalId, ClockRef, ClockStamp, Flags, GOD, ObjId, ObjKind, World,
};
pub use mush_tactics::field_ops::NpcMissionScope;
pub use runtime::{CrewHandles, action_descriptor, outcome_descriptor, register};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Staff-authored standing offer, representing the NPC's consent to these terms.
pub const OFFER_ATTR: &str = "crew:offer";
pub const MAX_CONTRACT_BYTES: usize = 4096;
const MAX_ROOM_CONTENTS: usize = 256;
const MAX_OFFERS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrewDuty {
    Mining,
    Combat,
    Recovery,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HireTerms {
    pub schema: u16,
    pub port: CanonicalId,
    pub mission_scope: NpcMissionScope,
    pub duties: BTreeSet<CrewDuty>,
}
impl HireTerms {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1 || self.duties.is_empty() || self.duties.len() > 3 {
            return Err("Invalid NPC crew terms.".into());
        }
        Ok(())
    }
    pub fn digest(&self) -> Result<CanonicalHash, String> {
        self.validate()?;
        let value = serde_json::to_value(self).map_err(|_| "Invalid NPC crew terms.")?;
        CanonicalHash::for_canonical_value("tacsim.crew_terms.v1", 1, &value, 2048)
            .map_err(|_| "Invalid NPC crew terms.".into())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrewContract {
    pub schema: u16,
    pub npc: ObjId,
    pub employer: ObjId,
    pub hired_at: ClockStamp,
    pub terms: HireTerms,
    pub terms_hash: CanonicalHash,
}
impl CrewContract {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || self.npc.0 == 0
            || self.employer.0 == 0
            || self.npc == self.employer
            || !matches!(self.hired_at.clock, ClockRef::WorldIc { generation } if generation > 0)
            || self.terms.digest()? != self.terms_hash
        {
            return Err("Invalid NPC crew contract.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HireOffer {
    npc: ObjId,
    room: ObjId,
    terms: HireTerms,
    terms_hash: CanonicalHash,
}
fn present(world: &World, actor: ObjId, kind: ObjKind) -> Option<ObjId> {
    let object = world.get(actor)?;
    if object.kind != kind
        || object.flags.intersects(Flags::GOING | Flags::DARK)
        || mush_sim::deployed::is_custodied(world, actor)
    {
        return None;
    }
    let room = object.location?;
    world.get(room).filter(|room| {
        room.kind == ObjKind::Room
            && !room.flags.intersects(Flags::GOING | Flags::DARK)
            && room.contents.contains(&actor)
    })?;
    Some(room)
}
/// Inspect only the current port room, never all NPCs or an embarkation log.
fn offers(
    world: &World,
    actor: ObjId,
    mut contracted: impl FnMut(ObjId) -> Result<bool, String>,
) -> Result<Vec<HireOffer>, String> {
    if world
        .module_state::<crate::TacsimStore>()
        .is_none_or(|s| !s.enabled())
    {
        return Err("NPC hiring unavailable.".into());
    }
    let room = present(world, actor, ObjKind::Player).ok_or("No crew available here.")?;
    let contents = &world.get(room).ok_or("No crew available here.")?.contents;
    if contents.len() > MAX_ROOM_CONTENTS {
        return Err("Crew hiring room is too busy.".into());
    }
    let ports = crate::port::market::ports(world);
    let mut result = Vec::new();
    for npc in contents {
        if present(world, *npc, ObjKind::Npc) != Some(room) {
            continue;
        }
        let Some(attr) = world.get(*npc).and_then(|npc| npc.attrs.get(OFFER_ATTR)) else {
            continue;
        };
        if attr.owner != GOD {
            continue;
        }
        let Some(value) = attr.as_json() else {
            continue;
        };
        if mush_core::canonical_json_bytes_with_limit(value, 2048).is_err() {
            continue;
        }
        let Ok(terms) = serde_json::from_value::<HireTerms>(value.clone()) else {
            continue;
        };
        let Ok(terms_hash) = terms.digest() else {
            continue;
        };
        if !ports.iter().any(|(_, port, _)| {
            port.venue == terms.port
                && crate::port::ship::is_hangar_port(port)
                && world
                    .module_state::<mush_atlas::AtlasState>()
                    .is_some_and(|atlas| {
                        atlas
                            .records
                            .roomgeo
                            .get(&room)
                            .is_some_and(|geo| geo.key.pack == port.area)
                    })
        }) || contracted(*npc)?
        {
            continue;
        }
        result.push(HireOffer {
            npc: *npc,
            room,
            terms,
            terms_hash,
        });
        if result.len() > MAX_OFFERS {
            return Err("Crew hiring pool is too large.".into());
        }
    }
    result.sort_by_key(|offer| offer.npc);
    Ok(result)
}

#[cfg(test)]
mod tests;

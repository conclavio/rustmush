use super::model::*;
use crate::detachment::activation::{ActiveDetachment, Reservation};
use crate::detachment::crew::{CrewContract, personnel};
use crate::detachment::planning::projection;
use mush_core::{Flags, ObjId, ObjKind, World};
use mush_sim::{SpatialParent, SpatialState};
use mush_tactics::detachment::planning::PlannedController;
use serde_json::{Value, json};

pub(super) fn berth(
    world: &World,
    carrier: ObjId,
) -> Result<crate::surface::act::SiteBerth<'_>, String> {
    let view =
        crate::port::ship::hull_view(world, carrier).ok_or("Carrier interior unavailable.")?;
    if crate::port::voyage::voyage_blocks(world, carrier) {
        return Err("Cancel departure clearance before surface deployment.".into());
    }
    crate::surface::act::site_berth(world, &view)
        .map_err(|_| "Settle a bay-equipped carrier at a field site first.".into())
}
pub(super) fn live(world: &World, id: ObjId, kind: ObjKind) -> bool {
    world
        .get(id)
        .is_some_and(|o| o.kind == kind && !o.flags.contains(Flags::GOING))
}
pub(super) fn station(
    world: &World,
    operator: ObjId,
    unit: ObjId,
    npc: bool,
) -> Result<Value, String> {
    if !projection::physically_aboard(world, operator, unit) {
        return Err(
            "The operator must be inside the assigned unit at its real helm console.".into(),
        );
    }
    if !npc {
        return projection::physical_station(world, operator, unit)
            .and_then(|proof| serde_json::to_value(proof).ok())
            .ok_or_else(|| "Player's physical helm station changed.".into());
    }
    if !live(world, operator, ObjKind::Npc) {
        return Err("NPC operator unavailable.".into());
    }
    let spatial = world
        .module_state::<SpatialState>()
        .ok_or("Spatial state unavailable.")?;
    if spatial.is_quarantined(unit) || spatial.held_assignment(unit).is_some() {
        return Err("Unit station assignments unavailable.".into());
    }
    let stations = spatial.stations.get(&unit).ok_or("No unit helm station.")?;
    let assignments = spatial
        .assignments
        .get(&unit)
        .ok_or("No unit crew assignment.")?;
    let helm = mush_core::ChannelId::new("helm").ok_or("Helm channel unavailable.")?;
    let mut matches = Vec::new();
    for (id, assignment) in &assignments.seats {
        let Some(spec) = stations.stations.get(id) else {
            continue;
        };
        let Some(room) = spec.space else { continue };
        if assignment.actor == operator
            && assignment.check().is_ok()
            && spec.channels.contains(&helm)
            && assignment
                .granted
                .as_ref()
                .is_none_or(|channels| channels.contains(&helm))
            && world.get(operator).and_then(|o| o.location) == Some(room)
            && live(world, room, ObjKind::Room)
        {
            matches.push(json!({"station":id,"assignment":assignment,"spec":spec,
                "room":room,"generation":spatial.control_generation(unit)}));
        }
    }
    if matches.len() != 1 {
        return Err("NPC needs one real helm billet at its current console.".into());
    }
    matches.pop().ok_or_else(|| "NPC helm unavailable.".into())
}
pub(super) fn launch_person(
    world: &World,
    roster: &ActiveDetachment,
    slot: u16,
    contract: Option<CrewContract>,
) -> Result<Value, String> {
    let row = roster
        .plan
        .slots
        .iter()
        .find(|s| s.slot.get() == slot)
        .ok_or("Unknown roster slot.")?;
    let unit = row.unit.ok_or("No assigned unit.")?;
    let (operator, npc) = match row.controller {
        PlannedController::Npc { character } => {
            let terms = personnel::check(
                world,
                roster.organizer,
                roster.plan.carrier,
                character,
                roster.plan.npc_commander == Some(character),
                contract,
            )
            .map_err(|e| e.label())?;
            if !terms.duties.contains(&roster.duty)
                || terms.digest().ok().as_ref() != roster.npc_terms.get(&character)
            {
                return Err("NPC's accepted employment terms or duty changed.".into());
            }
            (character, true)
        }
        PlannedController::Player { character } if live(world, character, ObjKind::Player) => {
            (character, false)
        }
        _ => return Err("Assigned operator unavailable.".into()),
    };
    if !projection::physically_aboard(world, operator, roster.plan.carrier) {
        return Err("Operator is not currently aboard the selected carrier.".into());
    }
    station(world, operator, unit, npc)
}
pub(super) fn launch_unit(world: &World, unit: ObjId, carrier: ObjId) -> Result<Value, String> {
    let berth = berth(world, carrier)?;
    let spatial = world
        .module_state::<SpatialState>()
        .ok_or("Spatial state unavailable.")?;
    if !live(world, unit, ObjKind::Thing)
        || spatial.is_quarantined(unit)
        || !matches!(spatial.parent_of(unit), SpatialParent::Interior {space,slot:Some(ref slot)}
            if space == berth.bay_room && slot == &berth.bay_slot)
    {
        return Err("Unit must occupy the carrier's actual vehicle bay compartment.".into());
    }
    let unit = mush_tactics::module::unit_of(world, unit).ok_or("Tactical unit unavailable.")?;
    let condition = crate::field_ops::assess_tacsim_unit(unit, None).map_err(|e| e.to_string())?;
    if !condition.has_power || !condition.has_control || !condition.can_move {
        return Err(
            "Unit cannot launch under its own power; repair it or arrange physical recovery."
                .into(),
        );
    }
    Ok(
        json!({"revision":unit.revision(),"condition":condition,"parent":spatial.parent_of(unit.object())}),
    )
}
pub(super) fn in_bay(world: &World, flight: &Flight) -> bool {
    world.module_state::<SpatialState>().is_some_and(|s| matches!(s.parent_of(flight.key.unit),
        SpatialParent::Interior {space, slot:Some(ref slot)} if space == flight.bay_room && slot == &flight.bay_slot))
}
pub(super) fn on_site(world: &World, flight: &Flight) -> bool {
    world.module_state::<SpatialState>().is_some_and(|s| {
        matches!(s.parent_of(flight.key.unit),
        SpatialParent::Direct {domain,..} if domain == flight.domain)
    })
}
pub(super) fn at_ramp(world: &World, flight: &Flight) -> bool {
    let Some((domain, pose)) = mush_sim::pose_of(world, flight.key.unit) else {
        return false;
    };
    domain == flight.domain
        && (i128::from(pose.x_cm) - i128::from(flight.ramp.x_cm)).abs()
            <= i128::from(crate::surface::ground::SITE_CELL_CM)
        && (i128::from(pose.y_cm) - i128::from(flight.ramp.y_cm)).abs()
            <= i128::from(crate::surface::ground::SITE_CELL_CM)
}
pub(super) fn recover_unit(
    world: &World,
    roster: &ActiveDetachment,
    flight: &Flight,
) -> Result<Value, String> {
    let row = roster
        .plan
        .slots
        .iter()
        .find(|s| s.slot.get() == flight.slot)
        .ok_or("Unknown return slot.")?;
    let unit = mush_tactics::module::unit_of(world, flight.key.unit)
        .ok_or("Tactical unit unavailable.")?;
    let condition = crate::field_ops::assess_tacsim_unit(unit, None).map_err(|e| e.to_string())?;
    if !condition.has_power || !condition.has_control || !condition.can_move {
        return Err(
            "Unit cannot return under its own power; arrange physical towing and loading.".into(),
        );
    }
    let station = station(
        world,
        flight.operator,
        flight.key.unit,
        matches!(row.controller, PlannedController::Npc { .. }),
    )?;
    Ok(json!({"revision":unit.revision(),"condition":condition,"station":station}))
}
pub(super) fn reservation(
    roster: &ActiveDetachment,
    entity: ObjId,
    row: Option<Reservation>,
) -> Result<(), String> {
    if row.as_ref() != Some(&Reservation::for_entity(roster, entity)) {
        return Err("Accepted unit/person reservation changed.".into());
    }
    Ok(())
}
pub(super) fn pending_can_move(
    world: &World,
    roster: &ActiveDetachment,
    flight: &Flight,
    contract: Option<CrewContract>,
) -> bool {
    let Ok(berth) = berth(world, flight.carrier) else {
        return false;
    };
    if berth.entry != flight.site
        || berth.domain != flight.domain
        || berth.bay_room != flight.bay_room
        || berth.bay_slot != flight.bay_slot
    {
        return false;
    }
    if flight.phase == Phase::Departing {
        launch_unit(world, flight.key.unit, flight.carrier).is_ok()
            && launch_person(world, roster, flight.slot, contract).is_ok()
    } else {
        flight.phase == Phase::Returning
            && at_ramp(world, flight)
            && recover_unit(world, roster, flight).is_ok()
    }
}

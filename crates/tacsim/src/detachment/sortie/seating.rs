//! Explicit NPC boarding of an assigned unit, using the existing Sim writers.
use super::{OperationId, facts, runtime::SortieHandles};
use crate::detachment::planning::{PlanningHandles, projection};
use mush_core::{CmdError, Ctx, ObjId};
use mush_org::unit_authority::reader::{UnitAuthorityRequest, UnitAuthorityResponse};
use mush_sim::{SpatialParent, SpatialState, StationId};
use mush_tactics::detachment::planning::PlannedController;

pub(super) fn seat(ctx: &mut Ctx<'_>, operation: OperationId, slot: u16) -> Result<(), CmdError> {
    arrange(ctx, operation, slot).map_err(CmdError::msg)
}
fn arrange(ctx: &mut Ctx<'_>, operation: OperationId, slot: u16) -> Result<(), String> {
    let h = ctx
        .w
        .module_state::<SortieHandles>()
        .filter(|h| h.enabled)
        .cloned()
        .ok_or("NPC seating unavailable.")?;
    if operation.organizer != ctx.actor || ctx.authoritative_commit_pending() {
        return Err("NPC seating unavailable.".into());
    }
    let roster = ctx
        .action_state(&h.active, &operation.reference())
        .map_err(|e| e.to_string())?
        .filter(|r| r.active() && OperationId::of(r) == operation)
        .cloned()
        .ok_or("Accepted roster changed; inspect +sortie/active.")?;
    roster.validate()?;
    let row = roster
        .plan
        .slots
        .iter()
        .find(|row| row.slot.get() == slot)
        .ok_or("Unknown slot.")?;
    let PlannedController::Npc { character: npc } = row.controller else {
        return Err(
            "Only contracted NPC slots use +sortie/seat; players take their own stations.".into(),
        );
    };
    let unit = row.unit.ok_or("No assigned unit.")?;
    let contract = ctx
        .action_state(&h.contracts, &npc)
        .map_err(|e| e.to_string())?
        .cloned();
    let terms = crate::detachment::crew::personnel::check(
        ctx.w,
        ctx.actor,
        roster.plan.carrier,
        npc,
        roster.plan.npc_commander == Some(npc),
        contract,
    )
    .map_err(|e| e.label())?;
    if !terms.duties.contains(&roster.duty)
        || terms.digest().ok().as_ref() != roster.npc_terms.get(&npc)
        || !projection::physically_aboard(ctx.w, ctx.actor, roster.plan.carrier)
    {
        return Err("Board the selected carrier with the crew under the accepted contract.".into());
    }
    for entity in [unit, npc] {
        facts::reservation(
            &roster,
            entity,
            ctx.action_state(&h.reservations, &entity)
                .map_err(|e| e.to_string())?
                .cloned(),
        )?;
    }
    let reader = ctx
        .w
        .module_state::<PlanningHandles>()
        .and_then(|h| h.unit_authority_reader.as_ref())
        .ok_or("Unit authority unavailable.")?;
    if !matches!(
        ctx.read_owner(
            reader,
            UnitAuthorityRequest::CrewUse {
                unit,
                operator: npc
            }
        ),
        Ok(UnitAuthorityResponse::Authority { allowed: true })
    ) {
        return Err("Current Deploy and NPC OperateCrew permissions are required.".into());
    }
    if ctx
        .action_state(&h.heads, &unit)
        .map_err(|e| e.to_string())?
        .is_some()
    {
        let (_, flight) = super::prepare::current(&mut &*ctx, &h, unit)?;
        if flight.phase.open() {
            return Err("Unit already has an open flight.".into());
        }
    }
    let (station, room, assigned) = console(ctx, unit, roster.plan.carrier, npc)?;
    // These are ordinary physical/billet changes. A later refusal retains the
    // person's actual position; a retry never fabricates a completed lease.
    if ctx.w.get(npc).and_then(|person| person.location) != Some(room) {
        let from = ctx.w.get(npc).and_then(|person| person.location);
        mush_sim::place_interior(ctx, npc, room, None).map_err(|e| e.to_string())?;
        ctx.out.pin_deferred();
        ctx.emit_world_event(&mush_core::WorldEvent::Moved {
            who: npc,
            from,
            to: room,
        });
    }
    if !assigned {
        mush_sim::commands::assign_station(ctx, unit, &station, npc).map_err(|e| e.to_string())?;
    }
    let queued = mush_sim::station::request_npc_control(ctx, unit, &station, npc)
        .map_err(|e| e.to_string())?;
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.detachment",
        format!(
            "NPC #{} is at unit #{}'s {} console; {}. Launch checks the actual helm lease again.",
            npc.0,
            unit.0,
            station.as_str(),
            if queued {
                "control request queued"
            } else {
                "control already observed"
            }
        ),
    );
    Ok(())
}
fn console(
    ctx: &Ctx<'_>,
    unit: ObjId,
    carrier: ObjId,
    npc: ObjId,
) -> Result<(StationId, ObjId, bool), String> {
    let view =
        crate::port::ship::hull_view(ctx.w, carrier).ok_or("Carrier interior unavailable.")?;
    let (bay, room) = view.bay().ok_or("Carrier has no vehicle bay.")?;
    let state = ctx
        .w
        .module_state::<SpatialState>()
        .ok_or("Spatial state unavailable.")?;
    let slot = mush_sim::SlotId::new(bay.slot.id.as_str()).ok_or("Bay compartment unavailable.")?;
    if state.is_quarantined(unit)
        || state.held_assignment(unit).is_some()
        || !matches!(state.parent_of(unit),SpatialParent::Interior{space,slot:Some(ref at)} if space==room && at==&slot)
    {
        return Err("Unit must be in the selected carrier's actual vehicle bay.".into());
    }
    let stations = state
        .stations
        .get(&unit)
        .ok_or("Unit needs an authored physical helm console.")?;
    let mut found = Vec::new();
    for (id, spec) in &stations.stations {
        if spec.channels.len() != 1 || !spec.channels.iter().any(|c| c.as_str() == "helm") {
            continue;
        }
        let Some(room) = spec.space else { continue };
        let Some(space) = mush_sim::anchored_space_of(ctx.w, room) else {
            continue;
        };
        if !matches!(state.spaces.get(&space).map(|s|&s.anchor),Some(mush_sim::SpaceAnchor::MovingObject{host}) if *host==unit)
        {
            continue;
        }
        let occupant = state.assignments.get(&unit).and_then(|a| a.seats.get(id));
        if occupant.is_some_and(|a| a.actor != npc) {
            return Err("Release the existing helm billet first.".into());
        }
        found.push((id.clone(), room, occupant.is_some()));
    }
    if found.len() != 1 {
        return Err(
            "Unit needs exactly one authored physical helm console for this assignment.".into(),
        );
    }
    found
        .pop()
        .ok_or_else(|| "Unit console unavailable.".into())
}

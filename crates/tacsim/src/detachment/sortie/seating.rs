//! Explicit NPC boarding of an assigned unit, using the existing Sim writers.
use super::{OperationId, facts, runtime::SortieHandles};
use crate::detachment::planning::{PlanningHandles, projection};
use crate::port::boarding::console;
use mush_core::{CmdError, Ctx};
use mush_org::unit_authority::reader::{UnitAuthorityRequest, UnitAuthorityResponse};
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

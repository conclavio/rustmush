//! Explicit NPC boarding of an assigned unit, using the existing Sim writers.
use super::{OperationId, facts, runtime::SortieHandles};
use crate::detachment::planning::{PlanningHandles, projection};
use crate::port::boarding::console;
use mush_core::{CmdError, Ctx};
use mush_org::unit_authority::reader::{UnitAuthorityRequest, UnitAuthorityResponse};
use mush_tactics::detachment::planning::PlannedController;

/// **`+sortie/seat`'s refusal of a console that fires to a hire not on a combat duty** (doc 47 unit
/// CO.5, R-47-6). The activation's duty is one every hired NPC's contract carries
/// (`+sortie/activate`), so a contract without combat never reaches a console with the fire
/// channel; nor does a contract with combat activated for mining or recovery.
pub const NPC_SEAT_NO_FIRE: &str = "This crew member is not on a combat duty here, and that console fires. Seat them in an unarmed rig, or activate the plan for combat under a contract that covers it.";

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
    // Doc 47 unit CO.5 (R-47-6): hired crew may take an armed rig's one console, which carries the
    // helm, the sensors and the fire channel; Sim's hand-off then leases all three. The fire
    // channel is a combat duty's alone.
    if super::facts::console_fires(ctx.w, unit, &station) && !super::facts::fires_by_duty(&roster) {
        return Err(NPC_SEAT_NO_FIRE.into());
    }
    // M30: refuse re-seating an NPC who still holds a station billet on another host or is still
    // physically inside another unit, so the prior unit's helm billet cannot be orphaned.
    check_not_seated_elsewhere(ctx.w, npc, unit, roster.plan.carrier)?;
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

/// Ensure `npc` does not hold a station billet on another host or stand inside another unit before
/// moving them into `unit`, preventing orphaned helm billets and stuck leases (M30).
fn check_not_seated_elsewhere(
    w: &mush_core::World,
    npc: mush_core::ObjId,
    unit: mush_core::ObjId,
    carrier: mush_core::ObjId,
) -> Result<(), String> {
    if let Some(state) = w.module_state::<mush_sim::SpatialState>() {
        if let Some(other) = state
            .derived
            .assignments_by_actor
            .get(&npc)
            .into_iter()
            .flatten()
            .copied()
            .find(|host| *host != unit)
        {
            return Err(format!(
                "NPC #{} still holds a station billet on #{}; run +sortie/unseat #{} first.",
                npc.0, other.0, npc.0
            ));
        }
        // Fallback scan of `assignments` in case a host seat exists outside `derived`.
        if let Some(other) = state
            .assignments
            .iter()
            .filter(|(host, _)| **host != unit)
            .find(|(_, seats)| seats.seats.values().any(|seat| seat.actor == npc))
            .map(|(host, _)| *host)
        {
            return Err(format!(
                "NPC #{} still holds a station billet on #{}; run +sortie/unseat #{} first.",
                npc.0, other.0, npc.0
            ));
        }
        if let Some(here) = w.get(npc).and_then(|o| o.location)
            && let Some(space) = mush_sim::anchored_space_of(w, here)
            && let Some(mush_sim::SpaceAnchor::MovingObject { host }) =
                state.spaces.get(&space).map(|s| &s.anchor)
            && *host != unit
            && *host != carrier
        {
            return Err(format!(
                "NPC #{} is still inside unit #{}; run +sortie/unseat #{} first.",
                npc.0, host.0, npc.0
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::{GOD, ObjKind};

    #[test]
    fn m30_refuses_reseating_npc_with_existing_billet_or_inside_other_unit() {
        let mut tw =
            mush_testkit::TestWorld::with_modules(vec![
                Box::new(mush_sim::SimulationModule::new()),
            ]);
        let carrier = tw
            .world_mut()
            .create(ObjKind::Thing, "Carrier", GOD, Some(mush_core::LIMBO))
            .expect("carrier");
        let bay = tw
            .world_mut()
            .create(ObjKind::Room, "Bay", GOD, Some(carrier))
            .expect("bay");
        let unit_a = tw
            .world_mut()
            .create(ObjKind::Thing, "Rig A", GOD, Some(bay))
            .expect("unit a");
        let cab_a = tw
            .world_mut()
            .create(ObjKind::Room, "Cab A", GOD, Some(unit_a))
            .expect("cab a");
        let unit_b = tw
            .world_mut()
            .create(ObjKind::Thing, "Rig B", GOD, Some(bay))
            .expect("unit b");
        let npc = tw
            .world_mut()
            .create(ObjKind::Npc, "Mendes", GOD, Some(bay))
            .expect("npc");

        tw.exec_ok(GOD, &format!("@behavior/add #{}=sim.interior", bay.0));
        tw.exec_ok(GOD, &format!("@behavior/add #{}=sim.interior", cab_a.0));
        {
            let mut out = mush_core::Outbox::new();
            let engine = tw.engine_mut();
            let mut ctx = Ctx {
                w: &mut engine.world,
                actor: GOD,
                switches: Default::default(),
                args: Default::default(),
                out: &mut out,
                reg: &engine.registry,
                module: Some("sim".into()),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            mush_sim::run(
                &mut ctx,
                &mush_sim::Transition::Space {
                    space: bay,
                    anchor: mush_sim::SpaceAnchor::MovingObject { host: carrier },
                },
            )
            .expect("anchor bay");
            mush_sim::run(
                &mut ctx,
                &mush_sim::Transition::Space {
                    space: cab_a,
                    anchor: mush_sim::SpaceAnchor::MovingObject { host: unit_a },
                },
            )
            .expect("anchor cab_a");
        }
        // Case 1: clean NPC standing in carrier bay can be seated into unit_b.
        assert!(check_not_seated_elsewhere(tw.world(), npc, unit_b, carrier).is_ok());

        // Case 2: NPC holds a billet on unit_a -> refused for unit_b, allowed for retry on unit_a.
        tw.exec_ok(GOD, &format!("@spatial/station #{}=pilot,pilot", unit_a.0));
        tw.exec_ok(
            GOD,
            &format!("@spatial/assign #{}=pilot,#{}", unit_a.0, npc.0),
        );
        let err = check_not_seated_elsewhere(tw.world(), npc, unit_b, carrier)
            .expect_err("re-seating into unit_b must be refused while unit_a billet stands");
        assert!(err.contains("+sortie/unseat"), "{err}");
        assert!(check_not_seated_elsewhere(tw.world(), npc, unit_a, carrier).is_ok());

        // Case 3: NPC has no billet recorded but physically stands inside unit_a's cab -> refused for unit_b.
        tw.exec_ok(GOD, &format!("@spatial/unassign #{}=pilot", unit_a.0));
        tw.world_mut().move_to(npc, cab_a).expect("move into cab_a");
        let err = check_not_seated_elsewhere(tw.world(), npc, unit_b, carrier)
            .expect_err("re-seating into unit_b must be refused while inside unit_a");
        assert!(err.contains("+sortie/unseat"), "{err}");
        assert!(check_not_seated_elsewhere(tw.world(), npc, unit_a, carrier).is_ok());
    }
}

//! Physical rig boarding and leaving through ordinary Sim writers.
use crate::detachment::planning::{PlanningHandles, projection};
use mush_core::{Ctx, Flags, ObjId, ObjKind, World};
use mush_org::unit_authority::{
    UnitCapability,
    reader::{UnitAuthorityRequest, UnitAuthorityResponse},
};
use mush_sim::{SpatialParent, SpatialState, StationId};

pub(crate) fn bay(w: &World, unit: ObjId, carrier: ObjId) -> Result<ObjId, String> {
    let view = super::ship::hull_view(w, carrier).ok_or("Carrier interior unavailable.")?;
    let (bay, room) = view.bay().ok_or("Carrier has no vehicle bay.")?;
    let state = w
        .module_state::<SpatialState>()
        .ok_or("Spatial state unavailable.")?;
    let slot = mush_sim::SlotId::new(bay.slot.id.as_str()).ok_or("Bay compartment unavailable.")?;
    if state.is_quarantined(unit)
        || state.held_assignment(unit).is_some()
        || !matches!(state.parent_of(unit),SpatialParent::Interior{space,slot:Some(ref at)} if space==room && at==&slot)
    {
        return Err("Unit must be in the selected carrier's actual vehicle bay.".into());
    }
    Ok(room)
}

pub(crate) fn console(
    ctx: &Ctx<'_>,
    unit: ObjId,
    carrier: ObjId,
    actor: ObjId,
) -> Result<(StationId, ObjId, bool), String> {
    bay(ctx.w, unit, carrier)?;
    let state = ctx
        .w
        .module_state::<SpatialState>()
        .ok_or("Spatial state unavailable.")?;
    let stations = state
        .stations
        .get(&unit)
        .ok_or("Unit needs an authored physical helm console.")?;
    let mut found = Vec::new();
    for (id, spec) in &stations.stations {
        // Doc 32 cut CF.6c: an armed rig's one console carries the cockpit's sensors and fire
        // beside the helm, so it is the helm console that carries the helm.
        if !spec.channels.iter().any(|c| c.as_str() == "helm") {
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
        if occupant.is_some_and(|a| a.actor != actor) {
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

fn move_person(ctx: &mut Ctx<'_>, actor: ObjId, room: ObjId) -> Result<(), String> {
    // Sim's sanctioned interior placement performs its own live checks. The
    // generic exit-less MUSH move gate intentionally rejects placed occupants.
    let from = ctx.w.get(actor).and_then(|o| o.location);
    mush_sim::place_interior(ctx, actor, room, None).map_err(|e| e.to_string())?;
    ctx.out.pin_deferred();
    ctx.emit_world_event(&mush_core::WorldEvent::Moved {
        who: actor,
        from,
        to: room,
    });
    Ok(())
}

fn released(ctx: &mut Ctx<'_>, unit: ObjId, actor: ObjId, carrier: ObjId) -> Result<bool, String> {
    let h = ctx
        .w
        .module_state::<crate::detachment::sortie::SortieHandles>()
        .ok_or("Flight history unavailable.")?;
    if let Some(head) = ctx
        .action_state(&h.heads, &unit)
        .map_err(|e| e.to_string())?
    {
        head.validate()?;
        let flight = ctx
            .action_state(&h.flights, &head.flight)
            .map_err(|e| e.to_string())?
            .ok_or("Flight history unavailable.")?;
        flight.validate()?;
        if head.flight.unit != unit || flight.phase.open() {
            return Err("The unit still has an open flight. Wait for return confirmation or cancel its pending departure before leaving.".into());
        }
    }
    let reader = ctx
        .w
        .module_state::<crate::detachment::sortie::SortieHandles>()
        .and_then(|h| h.controls.clone())
        .ok_or("Crew control unavailable.")?;
    let held = ctx
        .read_owner(
            &reader,
            vec![mush_sim::crew_control::CrewControlQuery {
                unit,
                operator: actor,
            }],
        )
        .map_err(|e| e.to_string())?;
    match held.as_slice() {
        [false] => return Ok(true),
        [true] => {}
        _ => return Err("Crew control unavailable.".into()),
    }
    let (station, _, _) = console(ctx, unit, carrier, actor)?;
    mush_sim::station::request_control_release(ctx, unit, &station, actor)
        .map_err(|e| e.to_string())?;
    Ok(false)
}

fn vacate(ctx: &mut Ctx<'_>, actor: ObjId, unit: ObjId) -> Result<(), String> {
    let stations = ctx
        .w
        .module_state::<SpatialState>()
        .and_then(|s| s.assignments.get(&unit))
        .map(|a| {
            a.seats
                .iter()
                .filter(|(_, s)| s.actor == actor)
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for station in stations {
        mush_sim::station::vacate_station(ctx, unit, &station, actor).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Resolve the physically occupied rig and its containing carrier bay directly from
/// `SpatialState::parent_of(actor)` -> `SpaceAnchor::MovingObject { host }` (`P2-TSW-26`),
/// without requiring lessee ownership, bay locality, or a live `OperateCrew` grant.
fn occupied_rig_for_leave(w: &World, actor: ObjId) -> Result<(ObjId, ObjId, ObjId), String> {
    let state = w
        .module_state::<SpatialState>()
        .ok_or("Spatial state unavailable.")?;
    let SpatialParent::Interior { space, .. } = state.parent_of(actor) else {
        return Err("You are not inside that rig.".into());
    };
    let anchored = mush_sim::anchored_space_of(w, space).unwrap_or(space);
    let unit = match state.spaces.get(&anchored).map(|s| &s.anchor) {
        Some(mush_sim::SpaceAnchor::MovingObject { host })
            if super::ship::hull_view(w, *host).is_none() =>
        {
            *host
        }
        _ => return Err("You are not inside that rig.".into()),
    };
    let SpatialParent::Interior {
        space: bay_room, ..
    } = state.parent_of(unit)
    else {
        return Err("Unit must be in the selected carrier's actual vehicle bay.".into());
    };
    let (view, _) = super::ship::hull_of_room(w, bay_room)
        .ok_or("Unit must be in the selected carrier's actual vehicle bay.")?;
    let carrier = view.hull();
    let bay = bay(w, unit, carrier)?;
    Ok((unit, bay, carrier))
}

/// Only the lessee boards themself. The actual K6 control request remains a
/// separate ordinary player command after they choose their own focus.
pub(crate) fn player(ctx: &mut Ctx<'_>) -> Result<(), String> {
    if ctx.authoritative_commit_pending()
        || !ctx
            .w
            .get(ctx.actor)
            .is_some_and(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
    {
        return Err("Rig boarding unavailable.".into());
    }
    let usage = "Use +tv/<n> board or +tv/<n> leave while the rig is in the carrier's bay.";
    let raw = ctx.args.raw().trim();
    let leave = if ctx.switches.has("board") || ctx.switches.has("leave") {
        if !raw.is_empty() || (ctx.switches.has("board") && ctx.switches.has("leave")) {
            return Err(usage.into());
        }
        ctx.switches.has("leave")
    } else {
        match raw {
            "board" => false,
            "leave" => true,
            _ => return Err(usage.into()),
        }
    };
    let mut named = None;
    for s in ctx.switches.iter() {
        if s == "board" || s == "leave" {
            continue;
        }
        let n = s
            .parse::<u8>()
            .ok()
            .filter(|n| *n > 0 && u32::from(*n) <= crate::definitions::logistics::MAX_BAY_ITEMS)
            .ok_or(usage)?;
        if named.replace(n).is_some() {
            return Err(usage.into());
        }
    }
    // P2-TSW-26: when leaving a rig, resolve the physically occupied rig directly from
    // `SpatialState::parent_of(ctx.actor)` -> `SpaceAnchor::MovingObject { host }` without
    // requiring lessee ownership, bay locality, or a live `OperateCrew` grant, so delegated
    // operators (including after grant revocation) can never be trapped inside an exit-less cab.
    if leave {
        let actor = ctx.actor;
        let (unit, bay, carrier) = occupied_rig_for_leave(ctx.w, actor)?;
        if !projection::physically_aboard(ctx.w, actor, unit) {
            return Err("You are not inside that rig.".into());
        }
        if !released(ctx, unit, actor, carrier)? {
            super::coop::say(
                ctx,
                "Control release requested. Once it settles, repeat +tv/<n> leave to step into the bay.",
            );
            return Ok(());
        }
        move_person(ctx, actor, bay)?;
        vacate(ctx, actor, unit)?;
        super::coop::say(
            ctx,
            "You step into the vehicle bay and give back your rig billet.",
        );
        ctx.show_room(actor, bay);
        return Ok(());
    }
    let store = ctx
        .w
        .module_state::<crate::TacsimStore>()
        .ok_or("Rig leases unavailable.")?;
    let mut rentals = store
        .tv_leases_of(ctx.actor)
        .filter(|r| r.live() && named.is_none_or(|n| n == r.slot))
        .cloned()
        .collect::<Vec<_>>();
    // Doc 39 cut IN.9 (§4.13): or a rig titled to them (or an org they act for) riding a
    // carrier they have the run of, by its title view.
    if rentals.is_empty() {
        rentals = crate::port::owned::title_views_for(ctx.w, ctx.actor)
            .into_iter()
            .filter(|r| named.is_none_or(|n| n == r.slot))
            .collect();
    }
    // Doc 36a lane 3 (P1-6): or a live lease on the carrier whose bay the actor stands in, when
    // its lessee delegated OperateCrew to them (`+tv/<n>/permissions`, then `@unitgrant/issue`
    // within that loan); the lessee stays the lease's principal.
    if rentals.is_empty() {
        let here = ctx.w.get(ctx.actor).and_then(|o| o.location);
        let delegated: Vec<_> = store
            .tv_leases_live()
            .filter(|r| r.pilot != ctx.actor && named.is_none_or(|n| n == r.slot))
            .filter(|r| bay(ctx.w, r.tv, r.hull).ok() == here)
            .cloned()
            .collect();
        rentals = delegated
            .into_iter()
            .filter(|r| operate_crew(ctx, r.tv))
            .collect();
    }
    let [rental] = rentals.as_slice() else {
        return Err("Name one of your current rental rig slots.".into());
    };
    // A title view is never stored, so it carries no lease to check.
    if crate::port::owned::rig_row(ctx.w, rental.tv).is_none() {
        rental.check()?;
    }
    let unit = rental.tv;
    let bay = bay(ctx.w, unit, rental.hull)?;
    let actor = ctx.actor;
    let reader = ctx
        .w
        .module_state::<PlanningHandles>()
        .and_then(|h| h.unit_authority_reader.as_ref())
        .ok_or("Unit authority unavailable.")?;
    if !matches!(
        ctx.read_owner(
            reader,
            UnitAuthorityRequest::Authorize {
                unit,
                capability: UnitCapability::OperateCrew,
                delegate: false,
                mission: None,
            }
        ),
        Ok(UnitAuthorityResponse::Authority { allowed: true })
    ) {
        return Err(
            "Current OperateCrew permission is required; inspect +tv/<n>/permissions.".into(),
        );
    }
    let (station, room, assigned) = console(ctx, unit, rental.hull, actor)?;
    let here = ctx.w.get(actor).and_then(|o| o.location);
    if here != Some(bay) && here != Some(room) {
        return Err("Stand in the carrier's actual vehicle bay to board.".into());
    }
    if here != Some(room) {
        move_person(ctx, actor, room)?;
    }
    if !assigned {
        mush_sim::commands::assign_station(ctx, unit, &station, actor)
            .map_err(|e| e.to_string())?;
    }
    super::coop::say(
        ctx,
        format!(
            "You are at rig {}'s {} console. Choose +focus #{}/{} then @station/handoff #{}/{} to request control.",
            rental.slot,
            station.as_str(),
            unit.0,
            station.as_str(),
            unit.0,
            station.as_str()
        ),
    );
    ctx.show_room(actor, room);
    Ok(())
}

/// **Whether the actor holds a current OperateCrew permission on `unit`** (the owner read
/// `+tv/<n> board` makes; doc 36a lane 3, P1-6 reads it for a delegate too).
fn operate_crew(ctx: &Ctx<'_>, unit: ObjId) -> bool {
    let Some(reader) = ctx
        .w
        .module_state::<PlanningHandles>()
        .and_then(|h| h.unit_authority_reader.as_ref())
    else {
        return false;
    };
    matches!(
        ctx.read_owner(
            reader,
            UnitAuthorityRequest::Authorize {
                unit,
                capability: UnitCapability::OperateCrew,
                delegate: false,
                mission: None,
            }
        ),
        Ok(UnitAuthorityResponse::Authority { allowed: true })
    )
}

pub(crate) fn unseat_npc(
    ctx: &mut Ctx<'_>,
    npc: ObjId,
    h: &crate::detachment::crew::CrewHandles,
) -> Result<String, String> {
    if ctx.authoritative_commit_pending() {
        return Err("NPC unseating unavailable.".into());
    }
    let contract = ctx
        .action_state(&h.contracts, &npc)
        .map_err(|e| e.to_string())?
        .filter(|c| c.employer == ctx.actor && c.npc == npc)
        .ok_or("Crew contract unavailable.")?;
    contract.validate()?;
    let person = ctx
        .w
        .get(npc)
        .filter(|o| o.kind == ObjKind::Npc && !o.flags.contains(Flags::GOING))
        .ok_or("Crew unavailable.")?;
    let room = person.location.ok_or("Crew location unavailable.")?;
    let space = mush_sim::anchored_space_of(ctx.w, room).ok_or("Crew is not inside a unit.")?;
    let state = ctx
        .w
        .module_state::<SpatialState>()
        .ok_or("Spatial state unavailable.")?;
    let unit = match state.spaces.get(&space).map(|s| &s.anchor) {
        Some(mush_sim::SpaceAnchor::MovingObject { host }) => *host,
        _ => return Err("Crew is not inside a unit.".into()),
    };
    let SpatialParent::Interior {
        space: bay_room, ..
    } = state.parent_of(unit)
    else {
        return Err("The unit must return to the carrier's bay first.".into());
    };
    let (view, _) = super::ship::hull_of_room(ctx.w, bay_room)
        .ok_or("The unit must return to the carrier's bay first.")?;
    let carrier = view.hull();
    let bay = bay(ctx.w, unit, carrier)?;
    if !projection::physically_aboard(ctx.w, ctx.actor, carrier)
        || !projection::physically_aboard(ctx.w, npc, unit)
    {
        return Err("Board the actual carrier with your contracted crew first.".into());
    }
    if !released(ctx, unit, npc, carrier)? {
        return Ok(format!(
            "Control release requested for NPC #{}. Once it settles, repeat +sortie/unseat #{}.",
            npc.0, npc.0
        ));
    }
    move_person(ctx, npc, bay)?;
    vacate(ctx, npc, unit)?;
    Ok(format!(
        "NPC #{} steps into the carrier's vehicle bay and gives back their unit billet.",
        npc.0
    ))
}

/// Current room contents only, including people inside carried containers.
/// An unreadable or oversized local contents tree holds the rental for inspection.
pub(crate) fn occupied(w: &World, unit: ObjId) -> bool {
    let Some((_, row)) = super::vehicle::rig_row(w, unit) else {
        return true;
    };
    let mut todo = row.rooms.values().copied().map(ObjId).collect::<Vec<_>>();
    let mut seen = std::collections::BTreeSet::new();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        if seen.len() > 4096 {
            return true;
        }
        let Some(o) = w.get(id) else {
            return true;
        };
        if o.kind.is_actor() {
            return true;
        }
        if todo.len().saturating_add(o.contents.len()) > 4096 {
            return true;
        }
        todo.extend(o.contents.iter().copied());
    }
    false
}

/// Cross the carrier's internal cargo-hold/bay access, never its external ramp.
pub(crate) fn bay_access(ctx: &mut Ctx<'_>) -> Result<(), String> {
    if ctx.authoritative_commit_pending() || ctx.switches.iter().count() != 1 {
        return Err("Vehicle bay access unavailable.".into());
    }
    let here = ctx
        .w
        .get(ctx.actor)
        .and_then(|o| o.location)
        .ok_or("Board your carrier first.")?;
    let (view, _) = super::ship::hull_of_room(ctx.w, here).ok_or("Board your carrier first.")?;
    let hull = view.hull();
    let (_, bay) = view.bay().ok_or("Carrier has no vehicle bay.")?;
    let hold = view
        .row
        .rooms
        .get(&mush_core::CanonicalId::new("hold").map_err(|e| e.to_string())?)
        .copied()
        .map(ObjId)
        .ok_or("Carrier cargo hold unavailable.")?;
    let airlock = view.airlock().ok_or("Carrier airlock unavailable.")?;
    let rooms = view.row.rooms.values().copied().map(ObjId).collect();
    let to = match ctx.args.raw().trim() {
        "in" if here == hold => bay,
        "out" if here == bay => hold,
        _ => {
            return Err(
                "Use +tv/bay in from the cargo hold or +tv/bay out from the vehicle bay.".into(),
            );
        }
    };
    let actor = ctx.actor;
    if to == hold {
        // A walkable room is a member of the ship's airlock space, not a
        // separate Sim space. Re-enter that space and use its real exits.
        move_person(ctx, actor, airlock)?;
        super::passenger::walk(ctx, actor, hold, &rooms)?;
    } else {
        move_person(ctx, actor, to)?;
    }
    super::coop::say(
        ctx,
        format!(
            "Carrier #{} internal bay access: {}.",
            hull.0,
            if to == bay {
                "entered vehicle bay"
            } else {
                "returned to cargo hold"
            }
        ),
    );
    ctx.show_room(ctx.actor, to);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// **P2-TSW-26**: a delegated operator inside an exit-less rig cab resolves their occupied rig
    /// and bay directly from `SpatialState::parent_of(actor)` -> `SpaceAnchor::MovingObject`
    /// without lessee ownership, bay locality, or live `OperateCrew`; someone on the carrier's
    /// bridge is rejected as not inside a rig.
    #[test]
    fn p2_tsw_26_leave_resolves_occupied_rig_without_lessee_ownership_or_operate_crew() {
        use crate::port::owned::{Held, OwnedBody, OwnedHull};
        use crate::port::{HullRow, PortMap};
        use mush_core::CanonicalId;

        let mut tw = mush_testkit::TestWorld::with_modules(vec![
            Box::new(mush_sim::SimulationModule::new()),
            Box::new(crate::TacsimRuleset::default()),
        ]);
        let delegate = tw.spawn_player("Delegate");
        let carrier = tw
            .world_mut()
            .create(ObjKind::Thing, "Dray-P", ObjId(1), None)
            .expect("carrier");
        let bridge = tw
            .world_mut()
            .create(ObjKind::Room, "Bridge", ObjId(1), None)
            .expect("bridge");
        let bay_room = tw
            .world_mut()
            .create(ObjKind::Room, "Vehicle Bay", ObjId(1), None)
            .expect("bay");
        let rig = tw
            .world_mut()
            .create(ObjKind::Thing, "Rig", ObjId(1), None)
            .expect("rig");
        let cab = tw
            .world_mut()
            .create(ObjKind::Room, "Rig Cab", ObjId(1), None)
            .expect("cab");

        let lane_id = CanonicalId::new(crate::definitions::content::LANE_PACK).expect("lane id");
        let ceres_entry = CanonicalId::new("ceres_port").expect("entry");
        let dray_p = CanonicalId::new("tacsim.unit.dray_p").expect("dray_p");
        let lane_pack =
            crate::pack::load_pack(crate::definitions::content::LANE_JSON).expect("lane pack");
        let lane_ver = lane_pack.manifest().version();

        {
            let store = tw
                .world_mut()
                .module_state_mut::<crate::TacsimStore>()
                .expect("store");
            store.cfg.enabled = true;
            store
                .stage_pack(crate::LoadedPack {
                    source: "<test>".to_owned(),
                    pack: lane_pack,
                })
                .expect("stage");
            store.seeded_keys.insert((lane_id.clone(), lane_ver));
            store.applied.insert(lane_id.clone(), lane_ver);
            store.port_maps.insert(
                (lane_id.clone(), ceres_entry.clone()),
                PortMap::new(lane_id, ceres_entry.clone()),
            );
            let mut rooms = BTreeMap::new();
            rooms.insert(CanonicalId::new("bridge").expect("id"), bridge.0);
            rooms.insert(CanonicalId::new("bay").expect("id"), bay_room.0);
            let row = HullRow {
                n: 1,
                mark: "FPC-P-1".to_owned(),
                hull: carrier.0,
                rooms,
                exits: Vec::new(),
            };
            store.yard.owned.insert(
                carrier,
                Held::Present(Box::new(OwnedHull {
                    hull: carrier,
                    design: dray_p,
                    mark: "FPC-P-1".to_owned(),
                    home: ceres_entry,
                    provenance: 1,
                    body: OwnedBody::Ship(row),
                })),
            );
        }

        let rigs_slot = mush_sim::SlotId::new("rigs").expect("slot");
        {
            let engine = tw.engine_mut();
            let mut out = mush_core::Outbox::default();
            let mut ctx = Ctx {
                w: &mut engine.world,
                reg: &engine.registry,
                out: &mut out,
                actor: mush_core::GOD,
                args: Default::default(),
                switches: Default::default(),
                module: Some(crate::NS.into()),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            crate::port::hull::declare_space(
                &mut ctx,
                bridge,
                mush_sim::SpaceAnchor::MovingObject { host: carrier },
            )
            .expect("bridge space");
            crate::port::hull::declare_space(
                &mut ctx,
                bay_room,
                mush_sim::SpaceAnchor::MovingObject { host: carrier },
            )
            .expect("bay space");
            mush_sim::declare_compartment(&mut ctx, bay_room, rigs_slot.clone(), Some(2))
                .expect("rigs compartment");
            crate::port::hull::declare_space(
                &mut ctx,
                cab,
                mush_sim::SpaceAnchor::MovingObject { host: rig },
            )
            .expect("cab space");
            mush_sim::place_interior(&mut ctx, rig, bay_room, Some(rigs_slot))
                .expect("place rig in bay");
            mush_sim::place_interior(&mut ctx, delegate, cab, None).expect("place delegate in cab");
        }

        // Delegate has NO lease and NO OperateCrew grant, yet resolves `(rig, bay_room, carrier)`.
        let resolved = occupied_rig_for_leave(tw.world(), delegate)
            .expect("delegate inside rig cab must resolve rig and bay");
        assert_eq!(resolved, (rig, bay_room, carrier));

        // Someone standing on the carrier's bridge is NOT inside a rig.
        {
            let engine = tw.engine_mut();
            let mut out = mush_core::Outbox::default();
            let mut ctx = Ctx {
                w: &mut engine.world,
                reg: &engine.registry,
                out: &mut out,
                actor: mush_core::GOD,
                args: Default::default(),
                switches: Default::default(),
                module: Some(crate::NS.into()),
                typed_state: mush_core::ModuleActionState::unavailable(),
            };
            mush_sim::place_interior(&mut ctx, delegate, bridge, None)
                .expect("place delegate on bridge");
        }
        let err = occupied_rig_for_leave(tw.world(), delegate)
            .expect_err("carrier bridge occupant must not resolve as a rig");
        assert_eq!(err, "You are not inside that rig.");
    }
}

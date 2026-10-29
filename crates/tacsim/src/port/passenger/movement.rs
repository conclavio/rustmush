//! A booking buys a real berth. Walking uses ordinary exits; delivery requires
//! an arrived carrier journey, never an NPC's claim or a direct teleport.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use mush_core::{CanonicalId, Ctx, Flags, ObjId, ObjKind, World, WorldEvent};
use mush_sim::{JourneyBook, JourneyState};

use super::{economy, store, traveller};
use crate::port_tick::{Fire, Step, SwapRefused};
use store::{Booking, JourneyProof, Phase};

const MAX_WALK_ROOMS: usize = 128;

pub(crate) fn docked_at(w: &World, hull: ObjId, port: &str) -> bool {
    matches!(crate::port::ship::berth_of(w,hull), crate::port::ship::Berth::Docked { def, .. } if def.venue.as_str() == port)
}

fn station_rooms(w: &World, port: &str) -> BTreeSet<ObjId> {
    let Some((_, def, _)) = crate::port::market::ports(w)
        .into_iter()
        .find(|(_, def, _)| def.venue.as_str() == port)
    else {
        return BTreeSet::new();
    };
    w.module_state::<mush_atlas::AtlasState>()
        .map(|atlas| {
            atlas
                .records
                .roomgeo
                .values()
                .filter(|geo| geo.key.pack.as_str() == def.area.as_str())
                .map(|geo| geo.room)
                .filter(|id| {
                    w.get(*id)
                        .is_some_and(|o| o.kind == ObjKind::Room && !o.flags.contains(Flags::GOING))
                })
                .take(MAX_WALK_ROOMS)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn at_station(w: &World, person: ObjId, port: &str) -> bool {
    w.get(person)
        .and_then(|o| o.location)
        .is_some_and(|room| station_rooms(w, port).contains(&room))
}

fn captain_here(w: &World, row: &Booking) -> bool {
    crate::port::coop::helm_of(w, row.captain).is_some_and(|h| h.hull == row.hull)
        && (at_station(w, row.captain, &row.origin)
            || crate::port::voyage::aboard(w, row.hull, row.captain))
}

fn live_parties(w: &World, row: &Booking) -> bool {
    [row.npc, row.captain, row.hull]
        .iter()
        .all(|id| w.get(*id).is_some_and(|o| !o.flags.contains(Flags::GOING)))
}

/// Fresh world facts rechecked by the typed action's revision validator.
pub(crate) fn transition_allowed(w: &World, before: Option<&Booking>, next: &Booking) -> bool {
    if !live_parties(w, next) {
        return false;
    }
    match before {
        None => {
            let Some(def) = traveller(w, next.npc) else {
                return false;
            };
            def.key == next.traveller_key
                && def.origin == next.origin
                && def.destination == next.destination
                && next.fare_minor > 0
                && next.fare_minor <= def.max_fare_minor
                && captain_here(w, next)
                && docked_at(w, next.hull, &next.origin)
                && at_station(w, next.npc, &next.origin)
                && w.get(next.npc).and_then(|o| o.location)
                    == w.get(next.captain).and_then(|o| o.location)
                && !crate::port::voyage::voyage_blocks(w, next.hull)
                && w.is_connected(next.captain)
        }
        Some(row) => match next.phase {
            Phase::Aboard => {
                row.phase == Phase::Booked
                    && captain_here(w, row)
                    && docked_at(w, row.hull, &row.origin)
                    && crate::port::voyage::aboard(w, row.hull, row.npc)
                    && !crate::port::voyage::voyage_blocks(w, row.hull)
            }
            Phase::Delivered => {
                row.phase == Phase::Aboard
                    && crate::port::voyage::aboard(w, row.hull, row.npc)
                    && docked_at(w, row.hull, &row.destination)
                    && proof(w, row).as_ref() == next.journey.as_ref()
                    && next.journey.is_some()
            }
            Phase::Cancelled => {
                matches!(row.phase, Phase::Booked | Phase::Aboard)
                    && !crate::port::voyage::aboard(w, row.hull, row.npc)
                    && !crate::port::voyage::voyage_blocks(w, row.hull)
                    && if economy::replaceable(w, row) {
                        matches!(
                            crate::port::ship::berth_of(w, row.hull),
                            crate::port::ship::Berth::Docked { .. }
                        ) && crate::port::market::ports(w)
                            .into_iter()
                            .any(|(_, def, _)| at_station(w, row.npc, def.venue.as_str()))
                    } else {
                        captain_here(w, row)
                            && docked_at(w, row.hull, &row.origin)
                            && at_station(w, row.npc, &row.origin)
                    }
            }
            Phase::Booked => false,
        },
    }
}

fn proof(w: &World, row: &Booking) -> Option<JourneyProof> {
    let journey = w.module_state::<JourneyBook>()?.get(row.hull)?;
    let arrival_ic = journey.arrival_at()?;
    if !matches!(journey.state(), JourneyState::Arrived)
        || journey.departed().at < row.accepted_ic
        || arrival_ic > row.deadline_ic
        || !docked_at(w, row.hull, &row.destination)
    {
        return None;
    }
    Some(JourneyProof {
        route: journey.route().id().to_string(),
        departed_ic: journey.departed().at,
        arrival_ic,
    })
}

/// Shortest authored room route, bounded independently of world size. The actual
/// walk still checks locks, movement hooks and the live destination at every step.
fn path(w: &World, from: ObjId, to: ObjId, allowed: &BTreeSet<ObjId>) -> Option<Vec<ObjId>> {
    if !allowed.contains(&from) || !allowed.contains(&to) {
        return None;
    }
    let mut queue = VecDeque::from([from]);
    let mut previous = BTreeMap::<ObjId, (ObjId, ObjId)>::new();
    let mut visited = BTreeSet::from([from]);
    while let Some(room) = queue.pop_front() {
        if room == to {
            let mut route = Vec::new();
            let mut cursor = to;
            while cursor != from {
                let (prev, exit) = *previous.get(&cursor)?;
                route.push(exit);
                cursor = prev;
            }
            route.reverse();
            return Some(route);
        }
        for exit in w.exits_of(room).iter().take(32) {
            let Some(destination) = w
                .get(*exit)
                .filter(|o| !o.flags.contains(Flags::GOING))
                .and_then(|o| o.destination)
            else {
                continue;
            };
            if allowed.contains(&destination)
                && visited.len() < MAX_WALK_ROOMS
                && visited.insert(destination)
            {
                previous.insert(destination, (room, *exit));
                queue.push_back(destination);
            }
        }
    }
    None
}

fn walk(ctx: &mut Ctx<'_>, npc: ObjId, to: ObjId, allowed: &BTreeSet<ObjId>) -> Result<(), String> {
    let from = ctx
        .w
        .get(npc)
        .and_then(|o| o.location)
        .ok_or("The traveller has no location.")?;
    let route = path(ctx.w, from, to, allowed)
        .ok_or("The traveller cannot find a station route to the berth.")?;
    for exit in route {
        let expected = ctx
            .w
            .get(exit)
            .and_then(|o| o.destination)
            .ok_or("The route changed.")?;
        mush_core::commands::movement::traverse_exit(
            ctx.reg,
            ctx.w,
            npc,
            exit,
            ctx.out,
            ctx.typed_state.lens(),
        );
        ctx.out.pin_deferred();
        if ctx.w.get(npc).and_then(|o| o.location) != Some(expected) {
            return Err("The traveller was stopped on the way to the berth. Clear the route and invite them again.".into());
        }
    }
    Ok(())
}

fn own(ctx: &Ctx<'_>, npc: ObjId) -> Result<Booking, String> {
    store::list(ctx)?
        .into_iter()
        .find(|r| r.npc == npc && r.captain == ctx.actor)
        .ok_or_else(|| "You have no passage agreement with that traveller.".into())
}

pub(crate) fn board(ctx: &mut Ctx<'_>, npc: ObjId) -> Result<String, String> {
    let row = own(ctx, npc)?;
    if row.phase != Phase::Booked || !economy::active(ctx.w, &row) {
        return Err("The passage must be booked and its fare reserved before boarding.".into());
    }
    if !captain_here(ctx.w, &row)
        || !docked_at(ctx.w, row.hull, &row.origin)
        || crate::port::voyage::voyage_blocks(ctx.w, row.hull)
    {
        return Err(
            "Board at the agreed origin with your ship alongside, before filing clearance.".into(),
        );
    }
    if crate::port::voyage::aboard(ctx.w, row.hull, npc) {
        store::stage_transition(ctx, &row, Phase::Aboard, None)?;
        return Ok("The passenger is already aboard; the manifest is being confirmed.".into());
    }
    board_person(ctx, npc, row.hull, &row.origin)?;
    mush_npc::notify_role_event(
        ctx,
        npc,
        "You have boarded the agreed ship. Settle into the crew compartment; if someone is with you, you may thank them or make a little conversation about the trip.",
    );
    store::stage_transition(ctx, &row, Phase::Aboard, None)?;
    Ok(format!(
        "{} has come aboard {}. The passenger occupies a real berth and is included in the voyage's provisions.",
        crate::port::coop::name_of(ctx.w, npc),
        crate::port::coop::name_of(ctx.w, row.hull)
    ))
}

/// Shared physical boarding for an agreed passenger or hired crew member.
/// Callers establish their own agreement, captain and current-port checks.
/// This writes ordinary movement only, never a booking or crew manifest.
pub(crate) fn board_person(
    ctx: &mut Ctx<'_>,
    npc: ObjId,
    hull: ObjId,
    port: &str,
) -> Result<(), String> {
    let (gallery, airlock, slot, crew, rooms) = {
        let view =
            crate::port::ship::hull_view(ctx.w, hull).ok_or("The ship's berth is unavailable.")?;
        let crate::port::ship::Berth::Docked { map, .. } = crate::port::ship::berth_of(ctx.w, hull)
        else {
            return Err("The ship is not alongside.".into());
        };
        let gallery = map
            .presentation
            .map(ObjId)
            .ok_or("The berth gallery is unavailable.")?;
        let airlock = view.airlock().ok_or("The ship has no airlock.")?;
        let slot = crate::port::hull::berths_slot(view.port.programme, view.interior)
            .map_err(|e| e.to_string())?;
        let crew = CanonicalId::new("crew").ok().and_then(|id| view.room(&id));
        let rooms: BTreeSet<_> = view.row.rooms.values().copied().map(ObjId).collect();
        (gallery, airlock, slot, crew, rooms)
    };
    walk(ctx, npc, gallery, &station_rooms(ctx.w, port))?;
    mush_sim::place_interior(ctx, npc, airlock, Some(slot))
        .map_err(|_| "The ship has no available passenger berth.")?;
    ctx.out.pin_deferred();
    ctx.emit_world_event(&WorldEvent::Moved {
        who: npc,
        from: Some(gallery),
        to: airlock,
    });
    if let Some(crew) = crew {
        let _ = walk(ctx, npc, crew, &rooms);
    }
    Ok(())
}

fn ashore(ctx: &mut Ctx<'_>, row: &Booking, port: &str) -> Result<(), String> {
    if !crate::port::voyage::aboard(ctx.w, row.hull, row.npc) {
        return Ok(());
    }
    let gallery = match crate::port::ship::berth_of(ctx.w, row.hull) {
        crate::port::ship::Berth::Docked { map, def } if def.venue.as_str() == port => {
            map.presentation.map(ObjId)
        }
        _ => None,
    }
    .ok_or("The passenger can only leave alongside the agreed port.")?;
    crate::port::ship::put_ashore(
        ctx,
        row.npc,
        row.hull,
        gallery,
        "passenger disembarkation",
        |_| {},
    )?;
    ctx.out.pin_deferred();
    Ok(())
}

pub(crate) fn cancel(ctx: &mut Ctx<'_>, npc: ObjId) -> Result<String, String> {
    let row = own(ctx, npc)?;
    if !matches!(row.phase, Phase::Booked | Phase::Aboard)
        || !captain_here(ctx.w, &row)
        || !docked_at(ctx.w, row.hull, &row.origin)
        || crate::port::voyage::voyage_blocks(ctx.w, row.hull)
    {
        return Err("Cancel before departure, alongside the original port.".into());
    }
    economy::prepare_cancellation(ctx, &row)?;
    ashore(ctx, &row, &row.origin)?;
    store::stage_transition(ctx, &row, Phase::Cancelled, None)?;
    Ok("Cancellation filed. Any reserved fare returns to the traveller when the cancellation settles.".into())
}

/// Keep an agreed passenger aboard before departure, and preserve the durable
/// arrival until disembarkation. Queued departures share the same filing gate.
pub(crate) fn departure_refusal(ctx: &Ctx<'_>, hull: ObjId) -> Option<&'static str> {
    let rows = match store::list(ctx) {
        Ok(rows) => rows,
        Err(_) => {
            return Some("The passenger manifest is unavailable; try again when it is readable.");
        }
    };
    for row in rows.iter().filter(|r| r.hull == hull) {
        let aboard = crate::port::voyage::aboard(ctx.w, hull, row.npc);
        if (row.phase == Phase::Booked && aboard) || (row.phase == Phase::Aboard && !aboard) {
            return Some(
                "The passenger's boarding or cancellation is still being confirmed. Allow the manifest to settle before filing departure.",
            );
        }
        if row.phase == Phase::Booked {
            return Some(
                "Board your booked passenger or cancel the passage before filing departure.",
            );
        }
        if aboard && docked_at(ctx.w, hull, &row.destination) {
            return Some(
                "A passenger has reached their destination and is waiting to disembark. Allow the arrival to finish before filing another departure.",
            );
        }
    }
    None
}

/// Role events are presentation only. These callbacks never settle an agreement.
pub(crate) fn voyage_event(ctx: &mut Ctx<'_>, hull: ObjId, text: &str) {
    for row in store::list_world(ctx.w)
        .into_iter()
        .filter(|r| r.hull == hull && r.phase == Phase::Aboard)
    {
        if crate::port::voyage::aboard(ctx.w, hull, row.npc) {
            mush_npc::notify_role_event(ctx, row.npc, text);
        }
    }
}

/// Purse setup and display hydration submit no authoritative action.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let travellers: Vec<_> = ctx
        .w
        .iter()
        .filter(|o| o.kind == ObjKind::Npc)
        .filter_map(|o| traveller(ctx.w, o.id).map(|def| (o.id, def)))
        .take(128)
        .collect();
    for (npc, def) in travellers {
        if let Err(error) = economy::ensure_purse(ctx, npc, &def) {
            tracing::debug!(npc=npc.0,%error,"passenger purse awaiting funding");
        }
    }
    let _ = store::refresh(ctx);
}

pub(crate) fn agents(ctx: &Ctx<'_>) -> Vec<(ObjId, ObjId, u64)> {
    store::list(ctx)
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.phase != Phase::Cancelled)
        .map(|r| (r.npc, r.captain, r.id))
        .collect()
}

/// One passenger participates in the port's normal fair, bounded agent rotation.
pub(crate) fn step(ctx: &mut Ctx<'_>, fire: &mut Fire, id: u64) -> Step {
    let Ok(Some(row)) = store::get(ctx, id) else {
        return Step::Refused;
    };
    let transition = if matches!(row.phase, Phase::Booked | Phase::Aboard)
        && economy::replaceable(ctx.w, &row)
    {
        // A refund ends the commercial commitment, even if the original captain
        // has since left or lost the helm. Put an aboard traveller safely ashore
        // at the actual berth before committing its terminal bookkeeping.
        let port = match crate::port::ship::berth_of(ctx.w, row.hull) {
            crate::port::ship::Berth::Docked { def, .. }
                if !crate::port::voyage::voyage_blocks(ctx.w, row.hull) =>
            {
                def.venue.as_str().to_owned()
            }
            _ => return Step::Held,
        };
        if let Err(error) = ashore(ctx, &row, &port) {
            tracing::warn!(booking=row.id,%error,"refunded passenger awaiting safe disembarkation");
            return Step::Held;
        }
        Some((Phase::Cancelled, None))
    } else if row.phase == Phase::Booked {
        if let Err(error) = economy::activate(ctx, &row) {
            tracing::debug!(booking=row.id,%error,"passenger escrow awaiting activation");
        }
        (crate::port::voyage::aboard(ctx.w, row.hull, row.npc) && economy::active(ctx.w, &row))
            .then_some((Phase::Aboard, None))
    } else if row.phase == Phase::Aboard {
        if economy::active(ctx.w, &row)
            && captain_here(ctx.w, &row)
            && docked_at(ctx.w, row.hull, &row.origin)
            && at_station(ctx.w, row.npc, &row.origin)
            && !crate::port::voyage::aboard(ctx.w, row.hull, row.npc)
            && !crate::port::voyage::voyage_blocks(ctx.w, row.hull)
        {
            // Physical disembarkation may already be durable when a crash loses
            // the queued cancellation. Finish that exact origin-side request.
            Some((Phase::Cancelled, None))
        } else {
            proof(ctx.w, &row).map(|proof| (Phase::Delivered, Some(proof)))
        }
    } else {
        None
    };
    if let Some((phase, proof)) = transition {
        return match fire.as_principal(ctx, row.npc, row.captain, |ctx| {
            store::stage_transition(ctx, &row, phase, proof)
        }) {
            Ok(Ok(())) => Step::Submitted,
            Ok(Err(error)) => {
                tracing::debug!(booking=row.id,%error,"passenger receipt awaiting reconciliation");
                Step::Refused
            }
            Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => {
                Step::Deferred
            }
            Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
        };
    }
    if row.phase == Phase::Delivered {
        if let Err(error) = ashore(ctx, &row, &row.destination) {
            tracing::warn!(booking=row.id,%error,"passenger awaiting disembarkation");
            return Step::Held;
        }
        if economy::settled(ctx.w, &row) {
            let text = format!(
                "Completed our passage to {}. The agreed fare was paid on arrival.",
                row.destination
            );
            let cause = format!("passenger:{}", row.id);
            match mush_npc::record_completed_work_experience(
                ctx,
                row.npc,
                row.captain,
                &cause,
                &text,
            ) {
                Ok(true) => {
                    crate::port_tick::notify(
                        ctx,
                        row.captain,
                        &format!(
                            "{} arrived at {} and paid the agreed fare of {}.",
                            crate::port::coop::name_of(ctx.w, row.npc),
                            row.destination,
                            crate::port::coop::money_minor(ctx, row.fare_minor)
                        ),
                    );
                    mush_npc::notify_role_event(
                        ctx,
                        row.npc,
                        "Your passage is complete and the agreed fare is paid. You are ashore at your destination; thank your captain if present, and remember the journey when you meet again.",
                    );
                    return Step::Worked;
                }
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(booking=row.id,%error,"passenger memory awaiting reconciliation")
                }
            }
        }
    }
    Step::Idle
}

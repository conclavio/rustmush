//! The travel commands (S2, 14-spatial-implementation-plan §3.2):
//!
//! ```text
//! +travel                        where you can go from here, and how long it takes
//! +travel <place>[/<mode>]       set out
//! +travel/status                 where you have got to
//! +travel/cancel                 turn back, or stop where you are
//! +stakeout <checkpoint>         watch a checkpoint on a road out of here
//! +stakeout/end                  give the watch up and come back
//! +intercept <traveler>          stop a traveler when it reaches you; hold a vehicle at your stop
//! +intercept/release <traveler>  let it go
//! @atlas/journey list|show|send|hold|release|abort   staff
//! ```
//!
//! All of it is `legacy-post-image`, family `atlas.journey`: a journey is a bounded
//! whole document, and nothing here takes part in authority or prepared mechanics.
//! `+travel` bare and `/status` read only. Knowledge (lens K): no reply here names a
//! traveler's destination to anyone but the traveler and staff.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::output::{display_name, name_span};
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ObjId, ObjKind, match_object};
use mush_proto::{NarrativeKind, OutputEvent, RichText};
use smol_str::SmolStr;

use crate::module::{AtlasState, NS, is_staff};
use crate::record::RecordKey;
use crate::travel::{
    self, Heading, HoldReason, JourneyId, JourneyState, Node, Route, TravelOption, Traveler, Way,
    about_real, node_name, options_from, position_line, resolve_connection, route_real_s,
    service_vehicle, vehicle_connection, vehicle_stop,
};

/// The words `@atlas/journey` understands.
pub const JOURNEY_VERBS: [&str; 6] = ["list", "show", "send", "hold", "release", "abort"];

pub(crate) fn register_travel(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+travel".into(),
            shape: ArgShape::Raw,
            switches: vec![SmolStr::new_static("status"), SmolStr::new_static("cancel")],
            help: "+travel                     where you can go from here, by what, and how long it\n\
                   \x20                           takes; the road leaves from a named room\n\
                   +travel <place>[/<mode>]    set out. You walk the road on the world clock and\n\
                   \x20                           arrive when it is time; a stake-out at a checkpoint\n\
                   \x20                           can stop you on the way\n\
                   +travel/status              where you have got to\n\
                   +travel/cancel              turn back if you have not passed a checkpoint, or stop\n\
                   \x20                           where you are\n\
                   A timed way (a farm road, a long stair) works the same: go through it and you set\n\
                   off. Nobody but you and staff is told where you are going."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_travel),
    )?;
    reg.command(
        CommandSpec {
            name: "+stakeout".into(),
            shape: ArgShape::Raw,
            switches: vec![SmolStr::new_static("end")],
            help: "+stakeout                   the checkpoints you could watch from here\n\
                   +stakeout <checkpoint>      go and watch a checkpoint on a road out of this\n\
                   \x20                           neighborhood (or of the vehicle you are aboard). Anything\n\
                   \x20                           that comes that way is stopped in front of you\n\
                   +stakeout/end               give the watch up and come back; whatever you were\n\
                   \x20                           holding goes on its way"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_stakeout),
    )?;
    reg.command(
        CommandSpec {
            name: "+intercept".into(),
            aliases: vec!["waylay".into()],
            shape: ArgShape::Raw,
            switches: vec![SmolStr::new_static("release")],
            help: "+intercept <traveler>       stop a traveler when it reaches you — at the checkpoint\n\
                   \x20                           you are watching, or here — or hold a vehicle standing\n\
                   \x20                           at your stop. You learn only that it is here, now\n\
                   +intercept/release <traveler>   let it go on"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_intercept),
    )
}

fn say(ctx: &mut Ctx<'_>, lines: Vec<String>) {
    let text = lines.join("\n");
    ctx.out
        .to_actor(ctx.actor, OutputEvent::Sys(RichText::tag(NS, text)));
}

fn emit_room(ctx: &mut Ctx<'_>, room: ObjId, subject: ObjId, text: RichText) {
    ctx.out.to_room_from(
        room,
        Vec::new(),
        subject,
        OutputEvent::Narrative {
            kind: NarrativeKind::Emit,
            actor: None,
            text,
        },
    );
}

fn state<'a>(ctx: &'a Ctx<'_>) -> Result<&'a AtlasState, CmdError> {
    ctx.state::<AtlasState>()
        .ok_or_else(|| CmdError::msg("The atlas is not composed on this server."))
}

fn here_of(ctx: &Ctx<'_>, who: ObjId) -> Result<ObjId, CmdError> {
    ctx.w
        .get(who)
        .and_then(|o| o.location)
        .ok_or_else(|| CmdError::msg("You are nowhere."))
}

fn matches_name(name: &str, needle: &str) -> bool {
    let name = name.to_ascii_lowercase();
    let needle = needle.trim().to_ascii_lowercase();
    !needle.is_empty() && (name == needle || name.starts_with(&needle) || name.contains(&needle))
}

// ---------------------------------------------------------------------------
// +travel
// ---------------------------------------------------------------------------

fn cmd_travel(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if ctx.switches.has("status") {
        return travel_status(ctx);
    }
    if ctx.switches.has("cancel") {
        let id = state(ctx)?
            .travel
            .journey_of(actor)
            .map(|j| j.id)
            .ok_or_else(|| CmdError::msg("You are not travelling."))?;
        travel::cancel(ctx, id, "you turned back", Some(actor)).map_err(CmdError::msg)?;
        return Ok(());
    }
    let raw = ctx.args.raw().trim().to_string();
    if state(ctx)?.travel.journey_of(actor).is_some() {
        return Err(CmdError::msg(
            "You are already under way (+travel/status; +travel/cancel to stop).",
        ));
    }
    let here = here_of(ctx, actor)?;
    let staff = is_staff(ctx.w, actor);
    // Stranded on a road: the way's two ends are the choices.
    if let Some((way, cursor, heading)) = stranded_on(ctx, actor, here)? {
        return travel_from_road(ctx, &raw, way, cursor, heading);
    }
    let options = {
        let st = state(ctx)?;
        options_from(st, actor, here, staff)
    };
    if raw.is_empty() {
        return list_options(ctx, here, &options);
    }
    let (dest, mode) = match raw.split_once('/') {
        Some((d, m)) => (d.trim().to_string(), Some(m.trim().to_ascii_lowercase())),
        None => (raw.clone(), None),
    };
    let candidates: Vec<&TravelOption> = options
        .iter()
        .filter(|o| matches_name(&o.place_name, &dest))
        .filter(|o| mode.as_ref().is_none_or(|m| o.modes.iter().any(|x| x == m)))
        .collect();
    if candidates.is_empty() {
        return Err(CmdError::msg(if mode.is_some() {
            format!("No way to {dest:?} by that from here. +travel lists what there is.")
        } else {
            format!("You know no way to {dest:?} from here. +travel lists what there is.")
        }));
    }
    let pick = {
        let st = state(ctx)?;
        choose(st, ctx.w, &candidates, mode.as_deref())
    };
    let Some(pick) = pick else {
        return Err(CmdError::msg("No way there."));
    };
    if pick.leaves_from != here {
        return Err(CmdError::msg(format!(
            "The {} leaves from {}.",
            pick.way_name,
            display_name(ctx.w, pick.leaves_from)
        )));
    }
    let mode = chosen_mode(pick, mode.as_deref());
    let (way, heading, route) = (
        Way::Connection(pick.connection.clone()),
        pick.heading,
        pick.route.clone(),
    );
    travel::depart(ctx, Traveler::Character(actor), way, heading, &mode, route)
        .map(|_| ())
        .map_err(CmdError::msg)
}

/// The mode a traveler takes on an option: the one asked for, else foot if the road
/// can be walked, else the road's first.
fn chosen_mode(o: &TravelOption, asked: Option<&str>) -> String {
    asked
        .map(str::to_string)
        .or_else(|| o.modes.iter().find(|m| *m == "foot").cloned())
        .or_else(|| o.modes.first().cloned())
        .unwrap_or_else(|| "foot".to_string())
}

/// The option to take among those to one place: with no mode named, a road you can
/// walk comes first (a ride is a ride); then the quicker; then by name, so two equal
/// roads pick the same one every time.
fn choose<'a>(
    st: &AtlasState,
    w: &mush_core::World,
    candidates: &[&'a TravelOption],
    asked: Option<&str>,
) -> Option<&'a TravelOption> {
    candidates.iter().copied().min_by_key(|o| {
        let m = chosen_mode(o, asked);
        (
            if asked.is_none() && m != "foot" { 1 } else { 0 },
            route_real_s(st, w, &o.route, &m),
            o.way_name.clone(),
        )
    })
}

fn list_options(ctx: &mut Ctx<'_>, here: ObjId, options: &[TravelOption]) -> Result<(), CmdError> {
    let mut lines = Vec::new();
    if options.is_empty() {
        lines.push("You know no road out of here.".to_string());
        if let Some(vehicle) = vehicle_here(ctx, here) {
            lines.push(format!(
                "The {} stands here; board it and it will take you.",
                display_name(ctx.w, vehicle)
            ));
        }
        say(ctx, lines);
        return Ok(());
    }
    lines.push("From here you can travel to:".to_string());
    let st = state(ctx)?;
    let mut rows: Vec<String> = Vec::new();
    for o in options {
        for mode in &o.modes {
            let secs = route_real_s(st, ctx.w, &o.route, mode);
            let from = if o.leaves_from == here {
                String::new()
            } else {
                format!("  (leaves from {})", display_name(ctx.w, o.leaves_from))
            };
            rows.push(format!(
                "  {:<24} by {:<8} via the {:<22} {}{from}",
                o.place_name,
                mode,
                o.way_name,
                about_real(secs)
            ));
        }
    }
    lines.extend(rows);
    lines.push("+travel <place>[/<mode>] sets out; +travel/status on the way.".to_string());
    say(ctx, lines);
    Ok(())
}

fn vehicle_here(ctx: &Ctx<'_>, here: ObjId) -> Option<ObjId> {
    let st = ctx.state::<AtlasState>()?;
    st.records
        .connections
        .values()
        .filter_map(|c| service_vehicle(ctx.w, c))
        .find(|v| vehicle_stop(st, ctx.w, *v) == Some(here))
}

fn travel_status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let here = here_of(ctx, actor)?;
    let line = {
        let st = state(ctx)?;
        if let Some(j) = st.travel.journey_of(actor) {
            Some(position_line(st, ctx.w, j))
        } else if let Some(j) = st.travel.journey_of(here) {
            Some(format!(
                "You are aboard the {}: {}",
                display_name(ctx.w, here),
                position_line(st, ctx.w, j)
            ))
        } else if let Some((way, cursor, _)) = st.travel.stranded.get(&actor).cloned()
            && st.travel.is_transit(here)
        {
            let name = match &way {
                Way::Connection(k) => st.records.connections.get(k).map(|c| c.name.clone()),
                Way::Portal(_) => None,
            }
            .unwrap_or_else(|| "the road".to_string());
            Some(format!(
                "You are stopped on the {name}, {cursor} checkpoint{} along; +travel names the ends you can go to.",
                if cursor == 1 { "" } else { "s" }
            ))
        } else {
            None
        }
    };
    match line {
        Some(line) => say(ctx, vec![line]),
        None => say(ctx, vec!["You are not travelling.".to_string()]),
    }
    Ok(())
}

/// A traveler standing in a transit room with a stranded position: the way, the node
/// index it stopped at, and the heading it had.
fn stranded_on(
    ctx: &Ctx<'_>,
    actor: ObjId,
    here: ObjId,
) -> Result<Option<(Way, u32, Heading)>, CmdError> {
    let st = state(ctx)?;
    if !st.travel.is_transit(here) {
        return Ok(None);
    }
    if let Some((way, cursor, heading)) = st.travel.stranded.get(&actor)
        && st
            .travel
            .transit
            .get(way.key())
            .is_some_and(|d| d.room == here)
    {
        return Ok(Some((way.clone(), *cursor, *heading)));
    }
    // A restart forgot where along the road they were: from the near end.
    let way = st
        .travel
        .transit
        .iter()
        .find(|(_, d)| d.room == here)
        .map(|(k, _)| k.clone())
        .ok_or_else(|| CmdError::msg("You are nowhere the atlas knows."))?;
    let way = if st.records.connections.contains_key(&way) {
        Way::Connection(way)
    } else {
        Way::Portal(way)
    };
    Ok(Some((way, 0, Heading::Forward)))
}

fn travel_from_road(
    ctx: &mut Ctx<'_>,
    raw: &str,
    way: Way,
    cursor: u32,
    heading: Heading,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let Way::Connection(key) = &way else {
        return Err(CmdError::msg(
            "Only the way back or forward: +travel/cancel turns you back.",
        ));
    };
    // Both ends, each with the route from the node the traveler stopped at.
    let ends: Vec<(String, Heading, Route)> = {
        let st = state(ctx)?;
        let conn = st
            .records
            .connections
            .get(key)
            .ok_or_else(|| CmdError::msg("The road you are on is gone."))?;
        let mut ends = Vec::new();
        for h in [Heading::Forward, Heading::Reverse] {
            let Ok(full) = resolve_connection(&st.records, conn, h) else {
                continue;
            };
            // The node index in this heading's order.
            let last = full.nodes.len().saturating_sub(1);
            let at = if h == heading {
                usize::try_from(cursor).unwrap_or(0)
            } else {
                last.saturating_sub(usize::try_from(cursor).unwrap_or(0))
            };
            if at >= last {
                continue;
            }
            let route = Route {
                name: full.name.clone(),
                nodes: full
                    .nodes
                    .get(at..)
                    .map(<[Node]>::to_vec)
                    .unwrap_or_default(),
                base_ic: full
                    .base_ic
                    .get(at..)
                    .map(<[u32]>::to_vec)
                    .unwrap_or_default(),
                revision: full.revision,
            };
            let end_name = route
                .nodes
                .last()
                .map(|n| node_name(&st.records, ctx.w, n))
                .unwrap_or_default();
            ends.push((end_name, h, route));
        }
        ends
    };
    if raw.is_empty() {
        let mut lines = vec!["From here on the road you can go to:".to_string()];
        let st = state(ctx)?;
        for (name, _, route) in &ends {
            lines.push(format!(
                "  {:<24} {}",
                name,
                about_real(route_real_s(st, ctx.w, route, "foot"))
            ));
        }
        say(ctx, lines);
        return Ok(());
    }
    let dest = raw.split('/').next().unwrap_or(raw).trim();
    let (_, h, route) = ends
        .into_iter()
        .find(|(name, _, _)| matches_name(name, dest))
        .ok_or_else(|| CmdError::msg(format!("No end of this road is called {dest:?}.")))?;
    travel::depart(ctx, Traveler::Character(actor), way, h, "foot", route)
        .map(|_| ())
        .map_err(CmdError::msg)
}

// ---------------------------------------------------------------------------
// +stakeout
// ---------------------------------------------------------------------------

/// The checkpoints `who` may watch from `here`: those on connections out of the
/// room's neighborhood, or on the connection of the vehicle they are aboard.
fn watchable(ctx: &Ctx<'_>, here: ObjId) -> Result<Vec<(RecordKey, String, String)>, CmdError> {
    let st = state(ctx)?;
    let staff = is_staff(ctx.w, ctx.actor);
    let mut out: Vec<(RecordKey, String, String)> = Vec::new();
    let conns: Vec<&crate::record::ConnectionRec> =
        if let Some(c) = vehicle_connection(st, ctx.w, here) {
            vec![c]
        } else if let Some(nb) = st.records.neighborhood_of_room(here) {
            st.records
                .connections_in(&nb.region)
                .filter(|c| {
                    c.endpoints.0.neighborhood.as_ref() == Some(&nb.key.pack)
                        || c.endpoints.1.neighborhood.as_ref() == Some(&nb.key.pack)
                })
                .collect()
        } else if let Some(cp) = st.travel.patch_at(here) {
            st.records
                .connections
                .values()
                .filter(|c| c.legs.iter().any(|l| &l.to == cp))
                .collect()
        } else {
            Vec::new()
        };
    for conn in conns {
        // K: a hidden road's checkpoints are offered only to someone who knows an end
        // of it, or to staff.
        let known = staff
            || conn.visibility == crate::record::Visibility::Public
            || [&conn.endpoints.0, &conn.endpoints.1].iter().any(|ap| {
                st.records
                    .neighborhoods
                    .values()
                    .find(|n| Some(&n.key.pack) == ap.neighborhood.as_ref())
                    .is_some_and(|n| st.travel.knows(ctx.actor, &n.key))
            });
        if !known {
            continue;
        }
        for leg in &conn.legs {
            let Some(cp) = st.records.checkpoints.get(&leg.to) else {
                continue;
            };
            match out.iter_mut().find(|(k, _, _)| k == &cp.key) {
                Some((_, _, ways)) if !ways.contains(&conn.name) => {
                    ways.push_str(", the ");
                    ways.push_str(&conn.name);
                }
                Some(_) => {}
                None => out.push((cp.key.clone(), cp.name.clone(), conn.name.clone())),
            }
        }
    }
    Ok(out)
}

fn cmd_stakeout(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if ctx.switches.has("end") {
        travel::end_watch(ctx, actor, true).map_err(CmdError::msg)?;
        say(ctx, vec!["You give up the watch.".to_string()]);
        return Ok(());
    }
    if state(ctx)?.travel.journey_of(actor).is_some() {
        return Err(CmdError::msg(
            "You are under way; you cannot watch a road from the road.",
        ));
    }
    let here = here_of(ctx, actor)?;
    let raw = ctx.args.raw().trim().to_string();
    let choices = watchable(ctx, here)?;
    if raw.is_empty() {
        let mut lines = Vec::new();
        if choices.is_empty() {
            lines.push("No road out of here has a checkpoint you could watch.".to_string());
        } else {
            lines.push("Checkpoints you could watch from here:".to_string());
            for (_, name, way) in &choices {
                lines.push(format!("  {name:<24} on the {way}"));
            }
            lines.push("+stakeout <checkpoint> goes and watches it.".to_string());
        }
        say(ctx, lines);
        return Ok(());
    }
    let (key, name, way) = choices
        .into_iter()
        .find(|(_, name, _)| matches_name(name, &raw))
        .ok_or_else(|| {
            CmdError::msg(format!(
                "No checkpoint called {raw:?} on a road out of here."
            ))
        })?;
    travel::begin_watch(ctx, actor, &key).map_err(CmdError::msg)?;
    say(
        ctx,
        vec![format!(
            "You settle in at {name}. Anything that comes along the {way} stops in front of you; \
             +stakeout/end gives it up."
        )],
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// +intercept
// ---------------------------------------------------------------------------

/// Find a traveler by what the actor typed: something here, or by name among the
/// Find a traveler by what the actor typed: something here that can be intercepted,
/// else by name among the live journeys and the service vehicles.
fn find_traveler(ctx: &Ctx<'_>, raw: &str) -> Result<ObjId, CmdError> {
    let st = state(ctx)?;
    // What can be intercepted at all: every live journey's traveler and every service
    // vehicle. A room's exit that happens to be called "bus" is neither.
    let candidates: Vec<ObjId> = st
        .travel
        .journeys
        .values()
        .map(|j| j.traveler.obj())
        .chain(
            st.records
                .connections
                .values()
                .filter_map(|c| service_vehicle(ctx.w, c)),
        )
        .collect();
    let matched = match_object(ctx.w, ctx.actor, raw).ok();
    if let Some(obj) = matched
        && candidates.contains(&obj)
    {
        return Ok(obj);
    }
    let by_name = candidates.iter().copied().find(|o| {
        ctx.w
            .get(*o)
            .is_some_and(|o| matches_name(mush_core::output::primary_name(&o.name), raw))
    });
    by_name
        .or(matched)
        .ok_or_else(|| CmdError::msg("It is not going anywhere."))
}

fn cmd_intercept(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let raw = ctx.args.raw().trim().to_string();
    if raw.is_empty() {
        return Err(CmdError::msg("Intercept what? +intercept <traveler>"));
    }
    let here = here_of(ctx, actor)?;
    let traveler = find_traveler(ctx, &raw)?;
    if traveler == actor {
        return Err(CmdError::msg(
            "You cannot stop yourself; +travel/cancel does that.",
        ));
    }
    let name = display_name(ctx.w, traveler);
    if ctx.switches.has("release") {
        return intercept_release(ctx, traveler, here);
    }
    // A vehicle standing at my stop: hold it here.
    let vehicle_conn = state(ctx)?
        .records
        .connections
        .values()
        .find(|c| service_vehicle(ctx.w, c) == Some(traveler))
        .map(|c| c.key.clone());
    if let Some(conn) = vehicle_conn {
        let standing_here = {
            let st = state(ctx)?;
            st.travel.journey_of(traveler).is_none()
                && vehicle_stop(st, ctx.w, traveler) == Some(here)
        };
        if standing_here {
            if let Some(st) = ctx.state_mut::<AtlasState>()
                && let Some(service) = st.travel.services.get_mut(&conn)
            {
                if service.held_by.is_some_and(|b| b != actor) {
                    return Err(CmdError::msg(format!(
                        "Someone is already holding the {name}."
                    )));
                }
                service.held_by = Some(actor);
            }
            let text = RichText::new().span(name_span(ctx.w, actor)).text(format!(
                " blocks the doors of the {name}; it is going nowhere."
            ));
            emit_room(ctx, here, actor, text);
            emit_room(
                ctx,
                traveler,
                actor,
                RichText::new().text(format!(
                    "Someone is blocking the doors; the {name} is held at the stop."
                )),
            );
            say(
                ctx,
                vec![format!(
                    "You hold the {name} here (+intercept/release lets it go)."
                )],
            );
            return Ok(());
        }
    }
    let (journey_id, held_here, under_way) = {
        let st = state(ctx)?;
        match st.travel.journey_of(traveler) {
            Some(j) => {
                let held_here = matches!(j.state, JourneyState::Held { .. })
                    && ctx.w.get(traveler).and_then(|o| o.location) == Some(here);
                (
                    Some(j.id),
                    held_here,
                    matches!(j.state, JourneyState::UnderWay),
                )
            }
            None => (None, false, false),
        }
    };
    let Some(_id) = journey_id else {
        return Err(CmdError::msg("It is not going anywhere."));
    };
    if held_here {
        return Err(CmdError::msg(format!(
            "The {name} is already stopped here."
        )));
    }
    let _ = under_way;
    // Where the hold lands: the checkpoint I watch, else this room.
    let at = {
        let st = state(ctx)?;
        match st.travel.watches.get(&actor) {
            Some(w)
                if st
                    .travel
                    .patches
                    .get(&w.checkpoint)
                    .is_some_and(|d| d.room == here) =>
            {
                Node::Checkpoint {
                    key: w.checkpoint.clone(),
                }
            }
            _ => Node::Room { room: here },
        }
    };
    if let Some(st) = ctx.state_mut::<AtlasState>() {
        st.travel
            .intercepts
            .insert(traveler, travel::Intercept { by: actor, at });
    }
    // K: no confirmation of whether it is coming this way at all.
    say(
        ctx,
        vec![format!(
            "You wait for the {name}: if it comes this way, you stop it."
        )],
    );
    Ok(())
}

fn intercept_release(ctx: &mut Ctx<'_>, traveler: ObjId, here: ObjId) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let name = display_name(ctx.w, traveler);
    // A vehicle I am holding at the stop.
    let held_conn = state(ctx)?
        .travel
        .services
        .iter()
        .find(|(k, s)| {
            s.held_by == Some(actor)
                && state(ctx)
                    .ok()
                    .and_then(|st| st.records.connections.get(*k))
                    .and_then(|c| service_vehicle(ctx.w, c))
                    == Some(traveler)
        })
        .map(|(k, _)| k.clone());
    if let Some(conn) = held_conn {
        if let Some(st) = ctx.state_mut::<AtlasState>()
            && let Some(service) = st.travel.services.get_mut(&conn)
        {
            service.held_by = None;
        }
        let text = RichText::new()
            .span(name_span(ctx.w, actor))
            .text(format!(" steps back from the doors of the {name}."));
        emit_room(ctx, here, actor, text);
        say(ctx, vec![format!("You let the {name} go.")]);
        return Ok(());
    }
    let id = {
        let st = state(ctx)?;
        let j = st
            .travel
            .journey_of(traveler)
            .ok_or_else(|| CmdError::msg("It is not going anywhere."))?;
        match &j.state {
            JourneyState::Held { by, .. } if by.by() == actor => j.id,
            JourneyState::Held { .. } => {
                return Err(CmdError::msg(format!(
                    "You are not the one holding the {name}."
                )));
            }
            _ => {
                // A pending intercept of mine is simply withdrawn.
                if st
                    .travel
                    .intercepts
                    .get(&traveler)
                    .is_some_and(|i| i.by == actor)
                {
                    JourneyId(u64::MAX)
                } else {
                    return Err(CmdError::msg(format!("The {name} is not stopped.")));
                }
            }
        }
    };
    if id == JourneyId(u64::MAX) {
        if let Some(st) = ctx.state_mut::<AtlasState>() {
            st.travel.intercepts.remove(&traveler);
        }
        say(ctx, vec![format!("You stop waiting for the {name}.")]);
        return Ok(());
    }
    travel::release(ctx, id, Some(actor)).map_err(CmdError::msg)?;
    say(ctx, vec![format!("You let the {name} go on.")]);
    Ok(())
}

// ---------------------------------------------------------------------------
// @atlas/journey (staff)
// ---------------------------------------------------------------------------

fn find_journey(ctx: &Ctx<'_>, raw: &str) -> Result<JourneyId, CmdError> {
    let st = state(ctx)?;
    if let Some(n) = raw.strip_prefix('J').and_then(|n| n.parse::<u64>().ok())
        && st.travel.journeys.contains_key(&JourneyId(n))
    {
        return Ok(JourneyId(n));
    }
    let obj = match_object(ctx.w, ctx.actor, raw)
        .ok()
        .or_else(|| {
            st.travel
                .journeys
                .values()
                .map(|j| j.traveler.obj())
                .find(|o| {
                    ctx.w.get(*o).is_some_and(|o| {
                        matches_name(mush_core::output::primary_name(&o.name), raw)
                    })
                })
        })
        .ok_or_else(|| CmdError::msg(format!("No journey for {raw:?}.")))?;
    st.travel
        .journey_of(obj)
        .map(|j| j.id)
        .ok_or_else(|| CmdError::msg(format!("{} is not under way.", display_name(ctx.w, obj))))
}

pub(crate) fn journey_admin(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if !is_staff(ctx.w, actor) {
        return Err(CmdError::msg("Permission denied."));
    }
    let raw = ctx.args.raw().trim().to_string();
    let (verb, rest) = raw
        .split_once(char::is_whitespace)
        .map_or((raw.as_str(), ""), |(v, r)| (v, r.trim()));
    match verb.to_ascii_lowercase().as_str() {
        "list" | "" => {
            let lines = {
                let st = state(ctx)?;
                let mut lines = vec![format!(
                    "{} journey{} under way:",
                    st.travel.journeys.len(),
                    if st.travel.journeys.len() == 1 {
                        ""
                    } else {
                        "s"
                    }
                )];
                for j in st.travel.journeys.values() {
                    lines.push(format!(
                        "  {:<5} {:<24} {}",
                        j.id.to_string(),
                        display_name(ctx.w, j.traveler.obj()),
                        position_line(st, ctx.w, j)
                    ));
                }
                for (conn, s) in &st.travel.services {
                    if let Some(c) = st.records.connections.get(conn) {
                        lines.push(format!(
                            "  service {:<20} vehicle at the {} end{}",
                            c.name,
                            match s.doc.at {
                                travel::End::From => "first",
                                travel::End::To => "second",
                            },
                            s.held_by
                                .map(|b| format!(", held by {}", display_name(ctx.w, b)))
                                .unwrap_or_default()
                        ));
                    }
                }
                // S9.2: the simulation's carrier journeys — a hull under way on
                // the placement graph along a corridor the atlas records — listed
                // beside the atlas's own, one view of everything travelling.
                for line in travel::carrier_lines(ctx.w) {
                    lines.push(line);
                }
                lines
            };
            say(ctx, lines);
            Ok(())
        }
        "show" => {
            let id = find_journey(ctx, rest)?;
            let lines = {
                let st = state(ctx)?;
                let j = st
                    .travel
                    .journeys
                    .get(&id)
                    .ok_or_else(|| CmdError::msg("No such journey."))?;
                let mut lines = vec![
                    format!(
                        "{} — {} ({:?}) by {} on {}",
                        j.id,
                        display_name(ctx.w, j.traveler.obj()),
                        j.traveler,
                        j.mode,
                        match &j.way {
                            Way::Connection(k) => format!("connection {}:{}", k.pack, k.entry),
                            Way::Portal(k) => format!("portal {}:{}", k.pack, k.entry),
                        }
                    ),
                    format!(
                        "  state {:?}; cursor {}/{}; heading {:?}",
                        j.state,
                        j.cursor,
                        j.legs_ic.len(),
                        j.heading
                    ),
                    format!(
                        "  departed at IC {} (generation {:?}); arrival at IC {}",
                        j.departed.at,
                        j.departed.clock,
                        j.arrival_at()
                            .map_or_else(|| "?".to_string(), |a| a.to_string())
                    ),
                ];
                for (i, n) in j.nodes.iter().enumerate() {
                    let leg = i
                        .checked_sub(1)
                        .and_then(|k| j.legs_ic.get(k))
                        .map(|l| format!("  (+{l}s IC)"))
                        .unwrap_or_default();
                    let mark = if usize::try_from(j.cursor).ok() == Some(i) {
                        "*"
                    } else {
                        " "
                    };
                    lines.push(format!(
                        "  {mark} {}{leg}",
                        node_name(&st.records, ctx.w, n)
                    ));
                }
                if let Some(a) = &j.anomaly {
                    lines.push(format!("  anomaly: {a}"));
                }
                lines
            };
            say(ctx, lines);
            Ok(())
        }
        "hold" => {
            let id = find_journey(ctx, rest)?;
            travel::hold_now(ctx, id, HoldReason::Staff { by: actor }).map_err(CmdError::msg)?;
            say(ctx, vec![format!("{id} held.")]);
            Ok(())
        }
        "release" => {
            let id = find_journey(ctx, rest)?;
            travel::release(ctx, id, Some(actor)).map_err(CmdError::msg)?;
            say(ctx, vec![format!("{id} released.")]);
            Ok(())
        }
        "send" => {
            // `send <object>=<place>[/<mode>]`: put a plain object on the road from the
            // room it stands in (the courier, the parcel, the black car's cargo).
            let (what, dest) = rest
                .split_once('=')
                .map(|(a, b)| (a.trim(), b.trim()))
                .ok_or_else(|| {
                    CmdError::msg("Usage: @atlas/journey send <object>=<place>[/<mode>]")
                })?;
            let obj = match_object(ctx.w, actor, what).map_err(|e| CmdError::msg(e.to_string()))?;
            let kind = ctx.w.get(obj).map(|o| o.kind);
            if !matches!(kind, Some(ObjKind::Thing)) {
                return Err(CmdError::msg(
                    "Only a plain object is sent; a person travels by +travel.",
                ));
            }
            let here = here_of(ctx, obj)?;
            let (place, mode) = match dest.split_once('/') {
                Some((d, m)) => (d.trim().to_string(), Some(m.trim().to_ascii_lowercase())),
                None => (dest.to_string(), None),
            };
            let options = {
                let st = state(ctx)?;
                options_from(st, actor, here, true)
            };
            let candidates: Vec<&TravelOption> = options
                .iter()
                .filter(|o| matches_name(&o.place_name, &place))
                .filter(|o| mode.as_ref().is_none_or(|m| o.modes.iter().any(|x| x == m)))
                .collect();
            let pick = {
                let st = state(ctx)?;
                choose(st, ctx.w, &candidates, mode.as_deref())
            }
            .ok_or_else(|| {
                CmdError::msg(format!(
                    "No road to {place:?} from where {} stands.",
                    display_name(ctx.w, obj)
                ))
            })?;
            let mode = chosen_mode(pick, mode.as_deref());
            let (way, heading, route) = (
                Way::Connection(pick.connection.clone()),
                pick.heading,
                pick.route.clone(),
            );
            let id = travel::depart(ctx, Traveler::Object(obj), way, heading, &mode, route)
                .map_err(CmdError::msg)?;
            say(
                ctx,
                vec![format!("{id}: {} is on its way.", display_name(ctx.w, obj))],
            );
            Ok(())
        }
        "abort" => {
            let (who, why) = rest
                .split_once(char::is_whitespace)
                .map_or((rest, "staff ended it"), |(w, r)| (w, r.trim()));
            let id = find_journey(ctx, who)?;
            travel::cancel(ctx, id, why, Some(actor)).map_err(CmdError::msg)?;
            say(ctx, vec![format!("{id} ended.")]);
            Ok(())
        }
        other => Err(CmdError::msg(format!(
            "@atlas/journey {other:?}? One of: {}.",
            JOURNEY_VERBS.join(", ")
        ))),
    }
}

/// Whether an object is a kind that travels as a character.
#[must_use]
pub fn is_character(ctx: &Ctx<'_>, obj: ObjId) -> bool {
    ctx.w
        .get(obj)
        .is_some_and(|o| matches!(o.kind, ObjKind::Player | ObjKind::Npc))
}

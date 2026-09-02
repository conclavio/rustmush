//! Staff commands: `@domain` and `@spatial` (design §12.2). Royalty+ or the
//! `SIM_ADMIN` power, checked in-handler (the faction idiom).

use std::sync::Arc;

use mush_core::output::display_name;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, Flags, ModError, ObjId, ObjKind, Powers, Registrar, Role,
    match_object, perm,
};
use smol_str::SmolStr;

use crate::data::{
    CapacitySpec, DomainClock, DomainId, DomainRecord, DomainStatus, HostAssignments, HostMounts,
    HostStations, MountId, MountSpec, PASSIVE_DRIVER, RECOVERY_LOG, RulesValue, SlotId, SpaceAnchor,
    SpatialParent, SpatialState, StationAssignment, StationId, StationSpec, persist_assignments,
    persist_del, persist_domain, persist_meta, persist_mounts, persist_space, persist_stations,
    space_key,
};
use crate::graph::{effective_domain, presence, project, verify_indexes, verify_projections};
use crate::links::{refresh_actor_links, refresh_unit_actors};
use crate::transition::{Transition, run};

fn is_admin(w: &mush_core::World, who: ObjId) -> bool {
    perm::has_power(w, who, Powers::SIM_ADMIN) || perm::has_role(w, who, Role::Royalty)
}

fn require_admin(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if is_admin(ctx.w, ctx.actor) {
        Ok(())
    } else {
        Err(CmdError::msg("Permission denied."))
    }
}

fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "sim", msg.into());
}

fn st<'a>(ctx: &'a Ctx<'_>) -> &'a SpatialState {
    ctx.w.module_state::<SpatialState>().expect("sim state slot installed at registration")
}

fn st_mut<'a>(ctx: &'a mut Ctx<'_>) -> &'a mut SpatialState {
    ctx.w.module_state_mut::<SpatialState>().expect("sim state slot installed at registration")
}

fn rhs_required(ctx: &Ctx<'_>, usage: &str) -> Result<String, CmdError> {
    ctx.args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CmdError::msg(format!("Usage: {usage}")))
}

/// Split a rhs on commas, but stop splitting at the first `{` so a trailing
/// JSON object survives intact. The JSON result is `Err` when a `{` is present
/// but does not parse (so a typo'd pose is reported, never silently dropped).
fn split_rhs(rhs: &str) -> (Vec<String>, Result<Option<serde_json::Value>, String>) {
    let (plain, json_str) = match rhs.find('{') {
        Some(i) => (&rhs[..i], Some(&rhs[i..])),
        None => (rhs, None),
    };
    let parts: Vec<String> = plain
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let json = match json_str {
        None => Ok(None),
        Some(j) => serde_json::from_str(j).map(Some).map_err(|e| format!("bad JSON pose: {e}")),
    };
    (parts, json)
}

fn pose_from(json: Option<serde_json::Value>) -> RulesValue {
    RulesValue {
        schema: SmolStr::new_static(PASSIVE_DRIVER),
        version: 1,
        data: json.unwrap_or(serde_json::Value::Null),
    }
}

/// Resolve `<id>` or a host object reference to a domain.
fn resolve_domain(ctx: &Ctx<'_>, s: &str) -> Result<DomainId, CmdError> {
    let s = s.trim();
    let state = st(ctx);
    // A bare number is a domain id; anything else (a name, a #dbref) is a host.
    if let Ok(n) = s.parse::<u64>() {
        return if state.domains.contains_key(&DomainId(n)) {
            Ok(DomainId(n))
        } else {
            Err(CmdError::msg(format!("No domain #{n}.")))
        };
    }
    let host = match_object(ctx.w, ctx.actor, s)?;
    state
        .domain_of_host(host)
        .map(|d| d.id)
        .ok_or_else(|| CmdError::msg("No such domain (give its id, or its host)."))
}

fn sim_err(e: crate::transition::SpatialError) -> CmdError {
    CmdError::msg(e.to_string())
}

// ----------------------------------------------------------------------
// @domain
// ----------------------------------------------------------------------

fn cmd_domain(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_admin(ctx)?;
    if ctx.switches.has("create") {
        domain_create(ctx)
    } else if ctx.switches.has("status") {
        domain_status(ctx)
    } else if ctx.switches.has("pause") {
        domain_set_status(ctx, DomainStatus::Paused)
    } else if ctx.switches.has("resume") {
        domain_set_status(ctx, DomainStatus::Active)
    } else if ctx.switches.has("retire") {
        domain_retire(ctx)
    } else if ctx.switches.has("present") {
        domain_present(ctx)
    } else {
        domain_list(ctx)
    }
}

fn domain_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lines: Vec<String> = st(ctx)
        .domains
        .values()
        .map(|d| {
            format!(
                "  #{} {:?} host {} presents in {} ({} local, {} projected)",
                d.id,
                d.status,
                display_name(ctx.w, d.host),
                display_name(ctx.w, d.presentation_location),
                st(ctx).derived.local_objects.get(&d.id).map_or(0, |s| s.len()),
                st(ctx).derived.projected.get(&d.id).map_or(0, |s| s.len()),
            )
        })
        .collect();
    if lines.is_empty() {
        info(ctx, "No domains.");
    } else {
        info(ctx, "Domains:");
        for l in lines {
            info(ctx, l);
        }
    }
    Ok(())
}

fn domain_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let host = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let rhs = rhs_required(ctx, "@domain/create <host>=<driver>[,<presentation room>]")?;
    let (parts, _) = split_rhs(&rhs);
    let driver = parts.first().cloned().unwrap_or_default();
    if driver != PASSIVE_DRIVER {
        return Err(CmdError::msg(format!("Unknown driver `{driver}` (cut 1 supports only `{PASSIVE_DRIVER}`).")));
    }
    let host_obj = ctx.w.get(host).ok_or_else(|| CmdError::msg("No such host."))?;
    if host_obj.flags.contains(Flags::GOING) {
        return Err(CmdError::msg("That host is being destroyed."));
    }
    if !ctx.w.has_behavior(host, &mush_core::BehaviorId::new("sim.domain").unwrap()) {
        return Err(CmdError::msg("The host must carry `sim.domain` first (@behavior/add)."));
    }
    if st(ctx).domain_of_host(host).is_some() {
        return Err(CmdError::msg("That host already has a domain."));
    }
    let presentation = match parts.get(1) {
        Some(p) => match_object(ctx.w, ctx.actor, p)?,
        None if host_obj.kind == ObjKind::Room => host,
        None => return Err(CmdError::msg("A non-room host needs an explicit presentation room.")),
    };
    if ctx.w.get(presentation).is_none_or(|o| o.kind != ObjKind::Room || o.flags.contains(Flags::GOING)) {
        return Err(CmdError::msg("The presentation location must be a live room."));
    }
    let (rec, next) = {
        let s = st_mut(ctx);
        let id = s.alloc_domain_id();
        let rec = DomainRecord {
            id,
            host,
            driver: SmolStr::new(driver),
            schema_version: 1,
            status: DomainStatus::Active,
            clock: DomainClock::Passive,
            presentation_location: presentation,
            revision: 1,
        };
        s.domains.insert(id, rec.clone());
        s.derived = crate::graph::rebuild(s);
        (rec, s.next_domain_id)
    };
    persist_domain(ctx.out, &rec);
    persist_meta(ctx.out, next);
    info(ctx, format!("Created domain #{} on {} (presents in {}).", rec.id, display_name(ctx.w, host), display_name(ctx.w, presentation)));
    Ok(())
}

fn domain_status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = resolve_domain(ctx, ctx.args.lhs())?;
    let lines: Vec<String> = {
        let s = st(ctx);
        let d = s.domains.get(&id).ok_or_else(|| CmdError::msg("No such domain."))?;
        let mut v = vec![format!(
            "Domain #{} — {:?}, driver {}, host {}, presents in {}, revision {}.",
            d.id, d.status, d.driver, display_name(ctx.w, d.host), display_name(ctx.w, d.presentation_location), d.revision
        )];
        let direct = s.derived.domain_direct.get(&id).map_or(0, |x| x.len());
        let local = s.derived.local_objects.get(&id).map_or(0, |x| x.len());
        let proj = s.derived.projected.get(&id).map_or(0, |x| x.len());
        let spaces = s.derived.spaces_in_domain.get(&id).map_or(0, |x| x.len());
        v.push(format!("  {direct} direct, {local} local, {proj} projected, {spaces} fixed space(s)."));
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn domain_set_status(ctx: &mut Ctx<'_>, status: DomainStatus) -> Result<(), CmdError> {
    let id = resolve_domain(ctx, ctx.args.lhs())?;
    let rec = {
        let s = st_mut(ctx);
        let d = s.domains.get_mut(&id).ok_or_else(|| CmdError::msg("No such domain."))?;
        if d.status == DomainStatus::Retired {
            return Err(CmdError::msg("That domain is retired."));
        }
        d.status = status;
        d.revision += 1;
        d.clone()
    };
    persist_domain(ctx.out, &rec);
    info(ctx, format!("Domain #{} is now {:?}.", rec.id, rec.status));
    Ok(())
}

fn domain_retire(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = resolve_domain(ctx, ctx.args.lhs())?;
    if st(ctx).domains.get(&id).is_some_and(|d| d.status == DomainStatus::Retired) {
        return Err(CmdError::msg(format!("Domain #{id} is already retired.")));
    }
    run(ctx, &Transition::Retire { domain: id }).map_err(sim_err)?;
    info(ctx, format!("Domain #{id} retired; its host may now drop `sim.domain`."));
    Ok(())
}

fn domain_present(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = resolve_domain(ctx, ctx.args.lhs())?;
    let room_s = rhs_required(ctx, "@domain/present <domain>=<room>")?;
    let room = match_object(ctx.w, ctx.actor, &room_s)?;
    if ctx.w.get(room).is_none_or(|o| o.kind != ObjKind::Room || o.flags.contains(Flags::GOING)) {
        return Err(CmdError::msg("The presentation location must be a live room."));
    }
    // Re-point, then re-project every direct participant.
    let rec = {
        let s = st_mut(ctx);
        let d = s.domains.get_mut(&id).ok_or_else(|| CmdError::msg("No such domain."))?;
        d.presentation_location = room;
        d.revision += 1;
        let rec = d.clone();
        s.derived = crate::graph::rebuild(s);
        rec
    };
    persist_domain(ctx.out, &rec);
    // Every direct participant AND its attached/interior subtree may project
    // through the presentation room (exposed riders follow their host).
    let moves: Vec<(ObjId, ObjId)> = {
        let s = st(ctx);
        s.derived
            .domain_direct
            .get(&id)
            .into_iter()
            .flat_map(|x| x.iter().copied())
            .flat_map(|root| s.subtree(root))
            .filter_map(|o| project(s, ctx.w, o).map(|dest| (o, dest)))
            .collect()
    };
    for (obj, dest) in moves {
        if ctx.w.get(obj).and_then(|o| o.location) != Some(dest)
            && let Err(e) = ctx.w.move_to(obj, dest)
        {
            info(ctx, format!("Could not re-project {}: {e}", display_name(ctx.w, obj)));
        }
    }
    info(ctx, format!("Domain #{} now presents in {}.", id, display_name(ctx.w, room)));
    Ok(())
}

// ----------------------------------------------------------------------
// @spatial
// ----------------------------------------------------------------------

fn cmd_spatial(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_admin(ctx)?;
    let sw = &ctx.switches;
    if sw.has("place") {
        spatial_place(ctx)
    } else if sw.has("unplace") {
        spatial_unplace(ctx)
    } else if sw.has("space") {
        spatial_space(ctx)
    } else if sw.has("unspace") {
        spatial_unspace(ctx)
    } else if sw.has("slot") {
        spatial_slot(ctx)
    } else if sw.has("mount") {
        spatial_mount(ctx)
    } else if sw.has("unmount") {
        spatial_unmount(ctx)
    } else if sw.has("station") {
        spatial_station(ctx)
    } else if sw.has("unstation") {
        spatial_unstation(ctx)
    } else if sw.has("assign") {
        spatial_assign(ctx)
    } else if sw.has("unassign") {
        spatial_unassign(ctx)
    } else if sw.has("egress") {
        spatial_egress(ctx)
    } else if sw.has("detach") {
        spatial_detach(ctx)
    } else if sw.has("quarantine") {
        spatial_quarantine_list(ctx)
    } else if sw.has("recover") {
        spatial_recover(ctx)
    } else if sw.has("validate") {
        spatial_validate(ctx)
    } else if sw.has("tree") {
        spatial_tree(ctx)
    } else if sw.has("log") {
        spatial_log(ctx)
    } else {
        spatial_show(ctx)
    }
}

fn describe_parent(ctx: &Ctx<'_>, p: &SpatialParent) -> String {
    match p {
        SpatialParent::Direct { domain, .. } => format!("direct in domain #{domain}"),
        SpatialParent::Interior { space, slot } => format!(
            "inside {}{}",
            display_name(ctx.w, *space),
            slot.as_ref().map_or(String::new(), |s| format!(" [{s}]"))
        ),
        SpatialParent::Attached { host, mount, .. } => format!("attached to {} at {mount}", display_name(ctx.w, *host)),
        SpatialParent::Unplaced => "unplaced".to_string(),
    }
}

fn spatial_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let lines: Vec<String> = {
        let s = st(ctx);
        let mut v = Vec::new();
        let name = display_name(ctx.w, obj);
        match s.placements.get(&obj) {
            None => v.push(format!("{name}: not a spatial entity.")),
            Some(p) => {
                v.push(format!("{name}: {} (revision {}).", describe_parent(ctx, &p.parent), p.revision));
                if let Some(u) = &p.unplaced {
                    v.push(format!(
                        "  {} — {} (previously {}).",
                        if u.quarantined { "QUARANTINED" } else { "unplaced" },
                        u.reason,
                        u.previous.as_ref().map_or("nothing".to_string(), |pp| describe_parent(ctx, pp))
                    ));
                }
                if let Some(d) = effective_domain(s, obj) {
                    v.push(format!("  effective domain #{d}; presence {:?}.", presence(s, obj)));
                }
            }
        }
        if let Some(d) = s.domain_of_host(obj) {
            v.push(format!("  hosts domain #{} ({:?}).", d.id, d.status));
        }
        if let Some(sp) = s.spaces.get(&obj) {
            v.push(format!(
                "  interior space, anchor {}; compartments: {}.",
                match &sp.anchor {
                    SpaceAnchor::FixedDomain { domain, .. } => format!("fixed in #{domain}"),
                    SpaceAnchor::MovingObject { host } => format!("rides {}", display_name(ctx.w, *host)),
                    SpaceAnchor::Unanchored => "none".into(),
                },
                if sp.compartments.is_empty() { "none".to_string() } else { sp.compartments.keys().map(ToString::to_string).collect::<Vec<_>>().join(", ") }
            ));
        }
        if let Some(m) = s.mounts.get(&obj) {
            v.push(format!("  mounts: {}.", m.mounts.iter().map(|(k, v)| format!("{k}{}", if v.exposed { " (exposed)" } else { "" })).collect::<Vec<_>>().join(", ")));
        }
        let kids = s.children_of(obj);
        if !kids.is_empty() {
            v.push(format!("  dependents: {}.", kids.iter().map(|&k| display_name(ctx.w, k)).collect::<Vec<_>>().join(", ")));
        }
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn spatial_tree(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let root = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let lines: Vec<String> = {
        let s = st(ctx);
        let mut v = Vec::new();
        fn walk(ctx: &Ctx<'_>, s: &SpatialState, obj: ObjId, depth: usize, v: &mut Vec<String>) {
            if depth > crate::data::MAX_SPATIAL_DEPTH {
                return;
            }
            v.push(format!("{}{} — {}", "  ".repeat(depth), display_name(ctx.w, obj), describe_parent(ctx, &s.parent_of(obj))));
            let mut kids: Vec<ObjId> = s.children_of(obj).iter().copied().collect();
            // A domain host also parents its directly placed participants.
            if let Some(d) = s.domain_of_host(obj)
                && let Some(direct) = s.derived.domain_direct.get(&d.id)
            {
                kids.extend(direct.iter().copied());
            }
            for c in kids {
                walk(ctx, s, c, depth + 1, v);
            }
        }
        walk(ctx, s, root, 0, &mut v);
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn spatial_place(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let rhs = rhs_required(ctx, "@spatial/place/domain <obj>=<domain>[,{pose}] | /place/interior <obj>=<space>[,<slot>] | /place/attached <obj>=<host>,<mount>[,{pose}]")?;
    let (parts, json) = split_rhs(&rhs);
    let json = json.map_err(CmdError::msg)?;
    let parent = if ctx.switches.has("domain") {
        let d = resolve_domain(ctx, parts.first().map(String::as_str).unwrap_or(""))?;
        SpatialParent::Direct { domain: d, pose: pose_from(json) }
    } else if ctx.switches.has("interior") {
        let space = match_object(ctx.w, ctx.actor, parts.first().map(String::as_str).unwrap_or(""))?;
        let slot = parts.get(1).and_then(|s| SlotId::new(s));
        SpatialParent::Interior { space, slot }
    } else if ctx.switches.has("attached") {
        let host = match_object(ctx.w, ctx.actor, parts.first().map(String::as_str).unwrap_or(""))?;
        let mount = parts.get(1).and_then(|m| MountId::new(m)).ok_or_else(|| CmdError::msg("Which mount? @spatial/place/attached <obj>=<host>,<mount>"))?;
        SpatialParent::Attached { host, mount, local_pose: json.map(|j| pose_from(Some(j))) }
    } else {
        return Err(CmdError::msg("Say how: @spatial/place/domain, /place/interior, or /place/attached."));
    };
    let desc = describe_parent(ctx, &parent);
    run(ctx, &Transition::Place { obj, parent }).map_err(sim_err)?;
    // A (re-)placed unit grants control to its crew again.
    refresh_unit_actors(ctx.w, obj);
    info(ctx, format!("Placed {} {desc}.", display_name(ctx.w, obj)));
    Ok(())
}

fn spatial_unplace(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let reason = ctx.args.rhs().map(str::trim).filter(|s| !s.is_empty()).unwrap_or("staff unplace").to_string();
    let keep = ctx.switches.has("keep");
    run(ctx, &Transition::Unplace { obj, reason, by: Some(ctx.actor), keep }).map_err(sim_err)?;
    // An unplaced unit stops granting control to its crew.
    refresh_unit_actors(ctx.w, obj);
    info(ctx, format!("Unplaced {}.", display_name(ctx.w, obj)));
    Ok(())
}

fn parse_anchor(ctx: &Ctx<'_>, s: &str) -> Result<SpaceAnchor, CmdError> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("unanchored") {
        return Ok(SpaceAnchor::Unanchored);
    }
    // The keyword is case-insensitive; the id/name after the colon is not.
    for (pfx, fixed) in [("fixed:", true), ("moving:", false)] {
        // Byte-length-safe and char-boundary-safe: the prefix is all ASCII, so
        // matching it means `pfx.len()` is a valid boundary for the tail slice.
        if s.as_bytes().get(..pfx.len()).is_some_and(|h| h.eq_ignore_ascii_case(pfx.as_bytes())) {
            let rest = s[pfx.len()..].trim();
            return if fixed {
                Ok(SpaceAnchor::FixedDomain { domain: resolve_domain(ctx, rest)?, anchor_pose: None })
            } else {
                Ok(SpaceAnchor::MovingObject { host: match_object(ctx.w, ctx.actor, rest)? })
            };
        }
    }
    Err(CmdError::msg("Anchor: fixed:<domain>, moving:<host>, or unanchored."))
}

fn spatial_space(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let space = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let anchor = parse_anchor(ctx, &rhs_required(ctx, "@spatial/space <room>=fixed:<domain>|moving:<host>|unanchored")?)?;
    if ctx.w.get(space).is_none_or(|o| o.kind != ObjKind::Room || o.flags.contains(Flags::GOING)) {
        return Err(CmdError::msg("An interior space must be a live room (cut 1)."));
    }
    if !ctx.w.has_behavior(space, &mush_core::BehaviorId::new("sim.interior").unwrap()) {
        return Err(CmdError::msg("The room must carry `sim.interior` first (@behavior/add)."));
    }
    // Define-or-re-anchor is one planned/applied transition: a refusal (bad
    // domain, cycle, anchor conflict) leaves no record behind.
    let existed = st(ctx).spaces.contains_key(&space);
    run(ctx, &Transition::Space { space, anchor }).map_err(sim_err)?;
    info(
        ctx,
        format!(
            "{} {}.",
            display_name(ctx.w, space),
            if existed { "re-anchored" } else { "is now an interior space" }
        ),
    );
    Ok(())
}

fn spatial_unspace(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let space = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    {
        let s = st(ctx);
        if !s.spaces.contains_key(&space) {
            return Err(CmdError::msg("That is not an interior space."));
        }
        let n = s.children_of(space).len();
        if n > 0 {
            return Err(CmdError::msg(format!("It still holds {n} occupant(s); move them first.")));
        }
        if s.is_placed(space) {
            return Err(CmdError::msg("Unplace the space itself first."));
        }
    }
    {
        let s = st_mut(ctx);
        s.spaces.remove(&space);
        s.derived = crate::graph::rebuild(s);
    }
    persist_del(ctx.out, space_key(space));
    info(ctx, format!("{} is no longer an interior space.", display_name(ctx.w, space)));
    Ok(())
}

fn spatial_slot(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let space = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let rhs = rhs_required(ctx, "@spatial/slot <space>=<slot>[,<max>]")?;
    let (parts, _) = split_rhs(&rhs);
    let slot = parts.first().and_then(|s| SlotId::new(s)).ok_or_else(|| CmdError::msg("Which slot?"))?;
    let max = parts.get(1).map(|m| m.parse::<u32>().map_err(|_| CmdError::msg("Max must be a number."))).transpose()?;
    let rec = {
        let s = st_mut(ctx);
        let sp = s.spaces.get_mut(&space).ok_or_else(|| CmdError::msg("That is not an interior space."))?;
        sp.compartments.insert(slot.clone(), CapacitySpec { maximum_items: max, ..Default::default() });
        sp.clone()
    };
    persist_space(ctx.out, &rec);
    info(ctx, format!("Compartment `{slot}` set on {}.", display_name(ctx.w, space)));
    Ok(())
}

fn spatial_mount(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let host = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let rhs = rhs_required(ctx, "@spatial/mount <host>=<mount>[,<max>][,exposed]")?;
    let (parts, _) = split_rhs(&rhs);
    let mount = parts.first().and_then(|m| MountId::new(m)).ok_or_else(|| CmdError::msg("Which mount?"))?;
    let mut max: Option<u32> = Some(1);
    let mut exposed = false;
    for p in parts.iter().skip(1) {
        if p.eq_ignore_ascii_case("exposed") {
            exposed = true;
        } else if let Ok(n) = p.parse::<u32>() {
            max = Some(n);
        } else {
            return Err(CmdError::msg(format!("Unknown mount option `{p}`.")));
        }
    }
    if ctx.w.get(host).is_none_or(|o| o.flags.contains(Flags::GOING)) {
        return Err(CmdError::msg("No such host."));
    }
    // A redefinition must not strand already-attached units above the new cap.
    if let Some(max) = max {
        let attached = st(ctx).attachments(host, &mount);
        if attached as u32 > max {
            return Err(CmdError::msg(format!(
                "{attached} unit(s) are mounted there; detach some before lowering the cap to {max}."
            )));
        }
    }
    let rec = {
        let s = st_mut(ctx);
        let m = s.mounts.entry(host).or_insert_with(|| HostMounts { host, mounts: Default::default() });
        m.mounts.insert(mount.clone(), MountSpec { capacity: CapacitySpec { maximum_items: max, ..Default::default() }, exposed });
        let rec = m.clone();
        // Exposure feeds the projected index and the units' projections.
        s.derived = crate::graph::rebuild(s);
        rec
    };
    persist_mounts(ctx.out, &rec);
    // Re-project whatever is already attached here (exposure may have flipped).
    let moves: Vec<(ObjId, ObjId)> = {
        let s = st(ctx);
        s.children_of(host)
            .iter()
            .copied()
            .filter(|&o| matches!(s.parent_of(o), SpatialParent::Attached { mount: ref m, .. } if *m == mount))
            .flat_map(|o| s.subtree(o))
            .filter_map(|o| project(s, ctx.w, o).map(|dest| (o, dest)))
            .collect()
    };
    for (obj, dest) in moves {
        if ctx.w.get(obj).and_then(|o| o.location) != Some(dest) {
            let _ = ctx.w.move_to(obj, dest);
        }
    }
    info(ctx, format!("Mount `{mount}` set on {}.", display_name(ctx.w, host)));
    Ok(())
}

fn spatial_unmount(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let host = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let mount = MountId::new(&rhs_required(ctx, "@spatial/unmount <host>=<mount>")?).ok_or_else(|| CmdError::msg("Which mount?"))?;
    if st(ctx).attachments(host, &mount) > 0 {
        return Err(CmdError::msg("Detach what is mounted there first."));
    }
    let rec = {
        let s = st_mut(ctx);
        let m = s.mounts.get_mut(&host).ok_or_else(|| CmdError::msg("No such mount."))?;
        if m.mounts.remove(&mount).is_none() {
            return Err(CmdError::msg("No such mount."));
        }
        let rec = m.clone();
        if rec.mounts.is_empty() {
            s.mounts.remove(&host);
        }
        rec
    };
    persist_mounts(ctx.out, &rec);
    info(ctx, format!("Mount `{mount}` removed from {}.", display_name(ctx.w, host)));
    Ok(())
}

fn spatial_quarantine_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lines: Vec<String> = st(ctx)
        .placements
        .values()
        .filter(|p| p.unplaced.as_ref().is_some_and(|u| u.quarantined))
        .map(|p| format!("  {} — {}", display_name(ctx.w, p.object), p.unplaced.as_ref().map_or("", |u| u.reason.as_str())))
        .collect();
    if lines.is_empty() {
        info(ctx, "Nothing is quarantined.");
    } else {
        info(ctx, "Quarantined:");
        for l in lines {
            info(ctx, l);
        }
    }
    Ok(())
}

fn spatial_recover(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let rec = {
        let s = st_mut(ctx);
        let p = s.placements.get_mut(&obj).filter(|p| p.unplaced.as_ref().is_some_and(|u| u.quarantined)).ok_or_else(|| CmdError::msg("That is not quarantined."))?;
        if let Some(u) = p.unplaced.as_mut() {
            u.quarantined = false;
            u.reason = format!("recovered by staff (was: {})", u.reason);
        }
        p.revision += 1;
        p.clone()
    };
    crate::data::persist_placement(ctx.out, &rec);
    info(ctx, format!("{} recovered; it is unplaced and may be re-placed.", display_name(ctx.w, obj)));
    Ok(())
}

fn spatial_validate(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (idx, bad) = {
        let s = st(ctx);
        (verify_indexes(s), verify_projections(s, ctx.w))
    };
    match idx {
        Ok(()) => info(ctx, "Indexes: consistent."),
        Err(e) => info(ctx, format!("Indexes: DRIFT — {e}")),
    }
    if bad.is_empty() {
        info(ctx, "Projections: consistent.");
    } else {
        for (obj, expected, actual) in bad {
            info(ctx, format!("Projection drift: {} should be in {} but is in {}.", display_name(ctx.w, obj), display_name(ctx.w, expected), actual.map_or("nowhere".to_string(), |a| display_name(ctx.w, a))));
        }
    }
    let q = st(ctx).placements.values().filter(|p| p.unplaced.as_ref().is_some_and(|u| u.quarantined)).count();
    info(ctx, format!("{q} quarantined."));
    Ok(())
}

fn spatial_log(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lines: Vec<String> = st(ctx)
        .recovery
        .open_entries(RECOVERY_LOG)
        .iter()
        .rev()
        .take(20)
        .map(|e| format!("  {}", e.value))
        .collect();
    if lines.is_empty() {
        info(ctx, "No recovery log entries in the open chunk.");
    } else {
        info(ctx, "Recovery log (newest first):");
        for l in lines {
            info(ctx, l);
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------
// Stations, assignments (cut 2)
// ----------------------------------------------------------------------

fn spatial_station(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let host = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let rhs = rhs_required(ctx, "@spatial/station <host>=<station>,<role>[,space:<room>]")?;
    let (parts, _) = split_rhs(&rhs);
    let station = parts.first().and_then(|s| StationId::new(s)).ok_or_else(|| CmdError::msg("Which station?"))?;
    let role = parts.get(1).cloned().filter(|r| !r.is_empty()).ok_or_else(|| CmdError::msg("Give a role (pilot, gunner, ...)."))?;
    let mut space: Option<ObjId> = None;
    for part in parts.iter().skip(2) {
        if let Some(room) = part.strip_prefix("space:") {
            space = Some(match_object(ctx.w, ctx.actor, room.trim())?);
        } else {
            return Err(CmdError::msg(format!("Unknown station option `{part}`.")));
        }
    }
    if ctx.w.get(host).is_none_or(|o| o.flags.contains(Flags::GOING)) {
        return Err(CmdError::msg("No such host."));
    }
    let rec = {
        let s = st_mut(ctx);
        let hs = s.stations.entry(host).or_insert_with(|| HostStations { host, stations: Default::default() });
        hs.stations.insert(
            station.clone(),
            StationSpec { role: SmolStr::new(&role), space, policy: RulesValue { schema: SmolStr::new_static(PASSIVE_DRIVER), version: 1, data: serde_json::Value::Null } },
        );
        hs.clone()
    };
    persist_stations(ctx.out, &rec);
    // A redefined role changes link priority; refresh any assigned crew.
    refresh_unit_actors(ctx.w, host);
    info(ctx, format!("Station `{station}` ({role}) set on {}.", display_name(ctx.w, host)));
    Ok(())
}

fn spatial_unstation(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let host = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let station = StationId::new(&rhs_required(ctx, "@spatial/unstation <host>=<station>")?).ok_or_else(|| CmdError::msg("Which station?"))?;
    if st(ctx).assignments.get(&host).is_some_and(|a| a.seats.contains_key(&station)) {
        return Err(CmdError::msg("Someone holds that station; @spatial/unassign first."));
    }
    let rec = {
        let s = st_mut(ctx);
        let hs = s.stations.get_mut(&host).ok_or_else(|| CmdError::msg("No such station."))?;
        if hs.stations.remove(&station).is_none() {
            return Err(CmdError::msg("No such station."));
        }
        let rec = hs.clone();
        if rec.stations.is_empty() {
            s.stations.remove(&host);
        }
        rec
    };
    persist_stations(ctx.out, &rec);
    info(ctx, format!("Station `{station}` removed from {}.", display_name(ctx.w, host)));
    Ok(())
}

fn spatial_assign(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let host = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let rhs = rhs_required(ctx, "@spatial/assign <host>=<station>,<actor>")?;
    let (parts, _) = split_rhs(&rhs);
    let station = parts.first().and_then(|s| StationId::new(s)).ok_or_else(|| CmdError::msg("Which station?"))?;
    let actor = match_object(ctx.w, ctx.actor, parts.get(1).map(String::as_str).unwrap_or(""))?;
    if ctx.w.get(actor).is_none_or(|o| o.flags.contains(Flags::GOING)) {
        return Err(CmdError::msg("No such actor."));
    }
    if !st(ctx).stations.get(&host).is_some_and(|hs| hs.stations.contains_key(&station)) {
        return Err(CmdError::msg("That host has no such station (@spatial/station first)."));
    }
    let now = ctx.w.now();
    {
        let s = st_mut(ctx);
        if let Some(a) = s.assignments.get(&host)
            && a.seats.get(&station).is_some_and(|seat| seat.actor != actor)
        {
            return Err(CmdError::msg("That station is already held; @spatial/unassign first."));
        }
        let ha = s.assignments.entry(host).or_insert_with(|| HostAssignments { host, seats: Default::default() });
        ha.seats.insert(station.clone(), StationAssignment { actor, assigned_at: now });
        let rec = ha.clone();
        s.derived = crate::graph::rebuild(s);
        persist_assignments(ctx.out, &rec);
    }
    refresh_actor_links(ctx.w, actor);
    info(ctx, format!("{} now holds `{station}` on {}.", display_name(ctx.w, actor), display_name(ctx.w, host)));
    Ok(())
}

fn spatial_unassign(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let host = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let station = StationId::new(&rhs_required(ctx, "@spatial/unassign <host>=<station>")?).ok_or_else(|| CmdError::msg("Which station?"))?;
    let former = {
        let s = st_mut(ctx);
        let ha = s.assignments.get_mut(&host).ok_or_else(|| CmdError::msg("No assignments on that host."))?;
        let asg = ha.seats.remove(&station).ok_or_else(|| CmdError::msg("No one holds that station."))?;
        let rec = ha.clone();
        if rec.seats.is_empty() {
            s.assignments.remove(&host);
        }
        s.derived = crate::graph::rebuild(s);
        persist_assignments(ctx.out, &rec);
        asg.actor
    };
    refresh_actor_links(ctx.w, former);
    info(ctx, format!("{} no longer holds `{station}` on {}.", display_name(ctx.w, former), display_name(ctx.w, host)));
    Ok(())
}

// ----------------------------------------------------------------------
// egress / detach (cut 2) -- Place with a computed parent
// ----------------------------------------------------------------------

/// The Direct parent an occupant egresses to: the space's effective domain
/// (read from the OCCUPANT while still inside) plus the chosen egress pose.
fn egress_parent(ctx: &Ctx<'_>, unit: ObjId, egress_id: Option<&str>) -> Result<SpatialParent, CmdError> {
    let s = st(ctx);
    let SpatialParent::Interior { space, .. } = s.parent_of(unit) else {
        return Err(CmdError::msg(format!("{} is not inside anything.", display_name(ctx.w, unit))));
    };
    let domain = effective_domain(s, unit).ok_or_else(|| CmdError::msg("That space has no domain to egress onto (it is unanchored)."))?;
    let rec = s.spaces.get(&space);
    let pose = match egress_id {
        Some(id) => Some(rec.and_then(|r| r.egresses.get(id)).ok_or_else(|| CmdError::msg(format!("No egress `{id}` on that space.")))?.to_pose.clone()),
        None => match rec.map(|r| &r.egresses) {
            Some(e) if e.len() == 1 => Some(e.values().next().unwrap().to_pose.clone()),
            Some(e) if e.len() > 1 => return Err(CmdError::msg("Which egress? Several are defined.")),
            _ => None,
        },
    };
    let pose = pose.flatten().unwrap_or(RulesValue { schema: SmolStr::new_static(PASSIVE_DRIVER), version: 1, data: serde_json::Value::Null });
    Ok(SpatialParent::Direct { domain, pose })
}

fn spatial_egress(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let unit = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let egress_id = ctx.args.rhs().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let parent = egress_parent(ctx, unit, egress_id.as_deref())?;
    run(ctx, &Transition::Place { obj: unit, parent }).map_err(sim_err)?;
    info(ctx, format!("{} egressed.", display_name(ctx.w, unit)));
    Ok(())
}

/// The parent a detached child takes: the host's OWN parent (become its sibling).
fn detach_parent(ctx: &Ctx<'_>, host: ObjId) -> Result<SpatialParent, CmdError> {
    match st(ctx).parent_of(host) {
        SpatialParent::Interior { space, .. } => Ok(SpatialParent::Interior { space, slot: None }),
        p @ (SpatialParent::Direct { .. } | SpatialParent::Attached { .. }) => Ok(p),
        SpatialParent::Unplaced => Err(CmdError::msg("The host is not placed; move it first.")),
    }
}

fn spatial_detach(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let child = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let SpatialParent::Attached { host, .. } = st(ctx).parent_of(child) else {
        return Err(CmdError::msg(format!("{} is not attached to anything.", display_name(ctx.w, child))));
    };
    let parent = detach_parent(ctx, host)?;
    run(ctx, &Transition::Place { obj: child, parent }).map_err(sim_err)?;
    info(ctx, format!("Detached {}.", display_name(ctx.w, child)));
    Ok(())
}

/// Player-facing `egress` / `disembark`: leave the interior you are in.
pub(crate) fn egress_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !matches!(st(ctx).parent_of(ctx.actor), SpatialParent::Interior { .. }) {
        return Err(CmdError::msg("You are not inside anything."));
    }
    let actor = ctx.actor;
    let parent = egress_parent(ctx, actor, None)?;
    run(ctx, &Transition::Place { obj: actor, parent }).map_err(sim_err)?;
    info(ctx, "You egress.");
    Ok(())
}

/// Register `@domain` and `@spatial`.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@domain".into(),
            shape: ArgShape::EqPair,
            switches: ["create", "status", "pause", "resume", "retire", "present"].iter().copied().map(SmolStr::new).collect(),
            help: "@domain  --  (staff) Simulation domains. No switch: list.\n\
                   @domain/create <host>=<driver>[,<presentation room>]   (driver: sim.passive)\n\
                   @domain/status <id|host>   @domain/pause|/resume <id|host>\n\
                   @domain/retire <id|host>   (must be unreferenced; then @behavior/remove sim.domain)\n\
                   @domain/present <id|host>=<room>   (re-point the presentation room)"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_domain),
    )?;
    reg.command(
        CommandSpec {
            name: "@spatial".into(),
            shape: ArgShape::EqPair,
            switches: [
                "place", "domain", "interior", "attached", "unplace", "keep", "space", "unspace", "slot",
                "mount", "unmount", "station", "unstation", "assign", "unassign", "egress",
                "detach", "quarantine", "recover", "validate", "tree", "log",
            ]
            .iter()
            .copied()
            .map(SmolStr::new)
            .collect(),
            help: "@spatial <obj>  --  (staff) Placement graph. Shows an object's placement.\n\
                   @spatial/tree <obj>\n\
                   @spatial/place/domain <obj>=<domain>[,{json pose}]\n\
                   @spatial/place/interior <obj>=<space>[,<slot>]\n\
                   @spatial/place/attached <obj>=<host>,<mount>[,{json}]\n\
                   @spatial/unplace[/keep] <obj>[=<reason>]\n\
                   @spatial/space <room>=fixed:<domain>|moving:<host>|unanchored   @spatial/unspace <room>\n\
                   @spatial/slot <space>=<slot>[,<max>]   @spatial/mount <host>=<mount>[,<max>][,exposed]   @spatial/unmount <host>=<mount>\n\
                   @spatial/station <host>=<station>,<role>[,space:<room>]   @spatial/unstation <host>=<station>\n\
                   @spatial/assign <host>=<station>,<actor>   @spatial/unassign <host>=<station>\n\
                   @spatial/egress <unit>[=<egress>]   @spatial/detach <child>   (embark/attach/transfer = @spatial/place/*)\n\
                   @spatial/quarantine   @spatial/recover <obj>   @spatial/validate   @spatial/log"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_spatial),
    )?;
    reg.command(
        CommandSpec {
            name: "egress".into(),
            aliases: vec!["disembark".into()],
            shape: ArgShape::Raw,
            help: "egress  --  Leave the interior space you are placed inside, onto the surrounding \
                   simulation domain (a mech egressing a dropship bay, a trooper leaving an APC)."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(egress_cmd),
    )?;
    Ok(())
}

//! Staff commands: `@domain` and `@spatial` (design §12.2). Royalty+ or the
//! `SIM_ADMIN` power, checked in-handler (the faction idiom).

use std::sync::Arc;

use mush_core::output::display_name;
use std::collections::BTreeSet;

use crate::data::{
    CapacitySpec, DomainClock, DomainId, DomainRecord, DomainStatus, HostAssignments, HostMounts,
    HostStations, MountId, MountSpec, PASSIVE_DRIVER, RECOVERY_LOG, RulesValue, SlotId,
    SpaceAnchor, SpatialCounterError, SpatialParent, SpatialState, StationAssignment, StationId,
    StationSpec, checked_domain_revision, checked_placement_revision, passive_rules_value,
    persist_assignments, persist_control_gen, persist_del, persist_domain, persist_meta,
    persist_mounts, persist_space, persist_stations, space_key,
};
use crate::domain::{
    self, ActivationBook, DomainActivation, DomainSummary, MAX_ACTIVE_DOMAINS, PromotionTrigger,
    QUIET_BOUNDARY,
};
use crate::focus::{AccountOracle, ActiveFocus, FocusState, MAX_BILLETS_LISTED, ReachProof};
use crate::graph::{effective_domain, presence, project, verify_indexes, verify_projections};
use crate::journey::{self, CarrierJourney, JourneyBook, JourneyId, JourneyState};
use crate::links::{refresh_actor_links, refresh_unit_actors};
use crate::route::{self, Journey, JourneyStatus, Leg, LegDef, MAX_LEGS, RouteDef, RouteState};
use crate::transition::{Transition, move_carrier_subtree, run};
use mush_core::{
    ArgShape, CanonicalId, ChannelId, ClockRef, ClockStamp, CmdError, CommandCompatibility,
    CommandSpec, Ctx, Flags, ModError, ObjId, ObjKind, Powers, Registrar, Role, match_object, perm,
};
use smol_str::SmolStr;

pub(crate) fn is_admin(w: &mush_core::World, who: ObjId) -> bool {
    perm::has_power(w, who, Powers::SIM_ADMIN) || perm::has_role(w, who, Role::Royalty)
}

fn require_admin(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if is_admin(ctx.w, ctx.actor) {
        Ok(())
    } else {
        Err(CmdError::msg("Permission denied."))
    }
}

pub(crate) fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "sim", msg.into());
}

pub(crate) fn st<'a>(ctx: &'a Ctx<'_>) -> &'a SpatialState {
    ctx.w
        .module_state::<SpatialState>()
        .expect("sim state slot installed at registration")
}

fn st_mut<'a>(ctx: &'a mut Ctx<'_>) -> &'a mut SpatialState {
    ctx.w
        .module_state_mut::<SpatialState>()
        .expect("sim state slot installed at registration")
}

/// The @spatial/station authoring default priority for a role (the demoted
/// former engine `role_priority` — now an overridable convenience, not engine
/// logic). Staff may pass `priority:<n>` to override.
fn default_station_priority(role: &str) -> u16 {
    match role {
        "pilot" => 2,
        "gunner" => 1,
        _ => 0,
    }
}

fn counter_err(error: SpatialCounterError) -> CmdError {
    CmdError::msg(error.to_string())
}

/// Preflight a unit's control-epoch bump, but ONLY if the object is actually a
/// control host (has stations or assignments) — so a plain placed object never
/// grows a spurious `cgen:` doc. This runs before a structural transition so an
/// exhausted epoch refuses with zero mutation.
fn prepare_control_bump_if_host(ctx: &Ctx<'_>, unit: ObjId) -> Result<Option<u64>, CmdError> {
    let is_host = {
        let s = st(ctx);
        s.stations.contains_key(&unit) || s.assignments.contains_key(&unit)
    };
    if is_host {
        st(ctx)
            .next_control_generation(unit)
            .map(Some)
            .map_err(counter_err)
    } else {
        Ok(None)
    }
}

/// Commit a previously checked control generation. No fallible work may occur
/// between preflight and this call on the single world thread.
fn apply_control_bump(ctx: &mut Ctx<'_>, unit: ObjId, next: Option<u64>) {
    if let Some(next) = next {
        st_mut(ctx).apply_control_generation(unit, next);
        persist_control_gen(ctx.out, unit, next);
    }
}

pub(crate) fn rhs_required(ctx: &Ctx<'_>, usage: &str) -> Result<String, CmdError> {
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
        Some(j) => serde_json::from_str(j)
            .map(Some)
            .map_err(|e| format!("bad JSON pose: {e}")),
    };
    (parts, json)
}

fn pose_from(json: Option<serde_json::Value>) -> Result<RulesValue, CmdError> {
    passive_rules_value(json.unwrap_or(serde_json::Value::Null))
        .map_err(|e| CmdError::msg(format!("Invalid passive rules value: {e}")))
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
    } else if ctx.switches.has("promote") {
        domain_promote(ctx)
    } else if ctx.switches.has("demote") {
        domain_demote(ctx)
    } else if ctx.switches.has("summary") {
        domain_summary(ctx)
    } else if ctx.switches.has("tick") {
        domain_tick(ctx)
    } else if ctx.switches.has("replay") {
        domain_replay(ctx)
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
                st(ctx)
                    .derived
                    .local_objects
                    .get(&d.id)
                    .map_or(0, |s| s.len()),
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
        return Err(CmdError::msg(format!(
            "Unknown driver `{driver}` (cut 1 supports only `{PASSIVE_DRIVER}`)."
        )));
    }
    let host_obj = ctx
        .w
        .get(host)
        .ok_or_else(|| CmdError::msg("No such host."))?;
    if host_obj.flags.contains(Flags::GOING) {
        return Err(CmdError::msg("That host is being destroyed."));
    }
    if !ctx
        .w
        .has_behavior(host, &mush_core::BehaviorId::new("sim.domain").unwrap())
    {
        return Err(CmdError::msg(
            "The host must carry `sim.domain` first (@behavior/add).",
        ));
    }
    if st(ctx).domain_of_host(host).is_some() {
        return Err(CmdError::msg("That host already has a domain."));
    }
    let presentation = match parts.get(1) {
        Some(p) => match_object(ctx.w, ctx.actor, p)?,
        None if host_obj.kind == ObjKind::Room => host,
        None => {
            return Err(CmdError::msg(
                "A non-room host needs an explicit presentation room.",
            ));
        }
    };
    if ctx
        .w
        .get(presentation)
        .is_none_or(|o| o.kind != ObjKind::Room || o.flags.contains(Flags::GOING))
    {
        return Err(CmdError::msg(
            "The presentation location must be a live room.",
        ));
    }
    let (rec, next) = {
        let s = st_mut(ctx);
        let id = s.alloc_domain_id().map_err(counter_err)?;
        let rec = DomainRecord {
            id,
            host,
            driver: crate::data::passive_definition_ref(),
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
    info(
        ctx,
        format!(
            "Created domain #{} on {} (presents in {}).",
            rec.id,
            display_name(ctx.w, host),
            display_name(ctx.w, presentation)
        ),
    );
    Ok(())
}

fn domain_status(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = resolve_domain(ctx, ctx.args.lhs())?;
    let lines: Vec<String> = {
        let s = st(ctx);
        let d = s
            .domains
            .get(&id)
            .ok_or_else(|| CmdError::msg("No such domain."))?;
        let mut v = vec![format!(
            "Domain #{} — {:?}, driver {}, host {}, presents in {}, revision {}.",
            d.id,
            d.status,
            PASSIVE_DRIVER,
            display_name(ctx.w, d.host),
            display_name(ctx.w, d.presentation_location),
            d.revision
        )];
        let direct = s.derived.domain_direct.get(&id).map_or(0, |x| x.len());
        let local = s.derived.local_objects.get(&id).map_or(0, |x| x.len());
        let proj = s.derived.projected.get(&id).map_or(0, |x| x.len());
        let spaces = s.derived.spaces_in_domain.get(&id).map_or(0, |x| x.len());
        v.push(format!(
            "  {direct} direct, {local} local, {proj} projected, {spaces} fixed space(s)."
        ));
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

/// Close a domain's standing activation because the domain is leaving
/// `Active` (E7 cut 6 fold, B3): write the summary FIRST — the same order
/// `@domain/demote` uses, so a reader can never see a stood-down domain with no
/// summary — then delete the activation and re-evaluate the queue.
///
/// Before the fold nothing but `@domain/demote` removed an activation, so a
/// retired or paused domain kept one of the eight tactical slots forever and
/// could not even be re-promoted to clear it (`domain_promote` refuses a
/// non-Active domain).
fn release_activation(ctx: &mut Ctx<'_>, id: DomainId, why: &str) {
    let Some(request) = ab(ctx).activations.get(&id).cloned() else {
        return;
    };
    let to = ctx.w.world_ic_now();
    let units = domain_units(ctx, id);
    let from = ClockStamp {
        clock: to.clock.clone(),
        at: request.at().at.min(to.at),
    };
    match DomainSummary::new(id, from, to, 0, units) {
        Ok(summary) => {
            if domain::persist_summary(ctx.out, &summary) {
                domain::persist_activation_del(ctx.out, id);
                if let Some(book) = ctx.w.module_state_mut::<ActivationBook>() {
                    book.summaries.insert(id, summary);
                    book.activations.remove(&id);
                }
                info(
                    ctx,
                    format!("Domain #{id} stood down ({why}); its summary is written first."),
                );
            }
        }
        Err(error) => tracing::error!(
            domain = %id,
            %error,
            "sim: a standing activation could not be closed; it keeps its tactical slot. \
             Recommended: @domain/demote the domain by hand"
        ),
    }
    for other in flush_dequeued(ctx) {
        info(ctx, format!("Domain #{other} came off the queue."));
    }
}

fn domain_set_status(ctx: &mut Ctx<'_>, status: DomainStatus) -> Result<(), CmdError> {
    let id = resolve_domain(ctx, ctx.args.lhs())?;
    let rec = {
        let s = st_mut(ctx);
        let d = s
            .domains
            .get_mut(&id)
            .ok_or_else(|| CmdError::msg("No such domain."))?;
        if d.status == DomainStatus::Retired {
            return Err(CmdError::msg("That domain is retired."));
        }
        let revision = checked_domain_revision(id, d.revision).map_err(counter_err)?;
        d.status = status;
        d.revision = revision;
        d.clone()
    };
    persist_domain(ctx.out, &rec);
    info(ctx, format!("Domain #{} is now {:?}.", rec.id, rec.status));
    if rec.status != DomainStatus::Active {
        release_activation(ctx, id, "it is no longer active");
    }
    Ok(())
}

fn domain_retire(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = resolve_domain(ctx, ctx.args.lhs())?;
    if st(ctx)
        .domains
        .get(&id)
        .is_some_and(|d| d.status == DomainStatus::Retired)
    {
        return Err(CmdError::msg(format!("Domain #{id} is already retired.")));
    }
    run(ctx, &Transition::Retire { domain: id }).map_err(sim_err)?;
    info(
        ctx,
        format!("Domain #{id} retired; its host may now drop `sim.domain`."),
    );
    release_activation(ctx, id, "it was retired");
    Ok(())
}

fn domain_present(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = resolve_domain(ctx, ctx.args.lhs())?;
    let room_s = rhs_required(ctx, "@domain/present <domain>=<room>")?;
    let room = match_object(ctx.w, ctx.actor, &room_s)?;
    if ctx
        .w
        .get(room)
        .is_none_or(|o| o.kind != ObjKind::Room || o.flags.contains(Flags::GOING))
    {
        return Err(CmdError::msg(
            "The presentation location must be a live room.",
        ));
    }
    let next_revision = {
        let d = st(ctx)
            .domains
            .get(&id)
            .ok_or_else(|| CmdError::msg("No such domain."))?;
        checked_domain_revision(id, d.revision).map_err(counter_err)?
    };
    // Plan: compute every re-projection under the NEW presentation room, and
    // validate them all BEFORE mutating — a refused move leaves the record and
    // the projections untouched (validate-then-apply, like the transitions).
    let moves: Vec<(ObjId, ObjId)> = {
        let s = st(ctx);
        let mut post = s.clone();
        if let Some(d) = post.domains.get_mut(&id) {
            d.presentation_location = room;
        }
        post.derived = crate::graph::rebuild(&post);
        let directs: Vec<ObjId> = post
            .derived
            .domain_direct
            .get(&id)
            .map(|x| x.iter().copied().collect())
            .unwrap_or_default();
        directs
            .iter()
            .flat_map(|root| post.subtree(*root))
            .filter_map(|o| project(&post, ctx.w, o).map(|dest| (o, dest)))
            .collect()
    };
    for (obj, dest) in &moves {
        if ctx.w.get(*obj).and_then(|o| o.location) != Some(*dest) {
            ctx.w.can_move_to(*obj, *dest).map_err(|e| {
                CmdError::msg(format!(
                    "Cannot re-project {}: {e}",
                    display_name(ctx.w, *obj)
                ))
            })?;
        }
    }
    // Apply: re-point the record, then perform the pre-validated moves.
    let rec = {
        let s = st_mut(ctx);
        let d = s
            .domains
            .get_mut(&id)
            .ok_or_else(|| CmdError::msg("No such domain."))?;
        d.presentation_location = room;
        d.revision = next_revision;
        let rec = d.clone();
        s.derived = crate::graph::rebuild(s);
        rec
    };
    persist_domain(ctx.out, &rec);
    for (obj, dest) in moves {
        if ctx.w.get(obj).and_then(|o| o.location) != Some(dest) {
            let _ = ctx.w.move_to(obj, dest);
        }
    }
    info(
        ctx,
        format!(
            "Domain #{} now presents in {}.",
            id,
            display_name(ctx.w, room)
        ),
    );
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
    } else if sw.has("authority") {
        spatial_authority(ctx)
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
        SpatialParent::Attached { host, mount, .. } => {
            format!("attached to {} at {mount}", display_name(ctx.w, *host))
        }
        SpatialParent::Unplaced => "unplaced".to_string(),
    }
}

fn spatial_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let lines: Vec<String> = {
        let s = st(ctx);
        let mut v = Vec::new();
        let name = display_name(ctx.w, obj);
        if let Some(held) = s.held_placement(obj) {
            v.push(format!(
                "{name}: HELD — unreadable authoritative {} ({})",
                held.key, held.reason
            ));
        } else {
            match s.placements.get(&obj) {
                None => v.push(format!("{name}: not a spatial entity.")),
                Some(p) => {
                    v.push(format!(
                        "{name}: {} (revision {}).",
                        describe_parent(ctx, &p.parent),
                        p.revision
                    ));
                    if let Some(u) = &p.unplaced {
                        v.push(format!(
                            "  {} — {} (previously {}).",
                            if u.quarantined {
                                "QUARANTINED"
                            } else {
                                "unplaced"
                            },
                            u.reason,
                            u.previous
                                .as_ref()
                                .map_or("nothing".to_string(), |pp| describe_parent(ctx, pp))
                        ));
                    }
                    if let Some(d) = effective_domain(s, obj) {
                        v.push(format!(
                            "  effective domain #{d}; presence {:?}.",
                            presence(s, obj)
                        ));
                    }
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
                    SpaceAnchor::MovingObject { host } =>
                        format!("rides {}", display_name(ctx.w, *host)),
                    SpaceAnchor::Unanchored => "none".into(),
                },
                if sp.compartments.is_empty() {
                    "none".to_string()
                } else {
                    sp.compartments
                        .keys()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ));
        }
        if let Some(m) = s.mounts.get(&obj) {
            v.push(format!(
                "  mounts: {}.",
                m.mounts
                    .iter()
                    .map(|(k, v)| format!("{k}{}", if v.exposed { " (exposed)" } else { "" }))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        let kids = s.children_of(obj);
        if !kids.is_empty() {
            v.push(format!(
                "  dependents: {}.",
                kids.iter()
                    .map(|&k| display_name(ctx.w, k))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
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
            v.push(format!(
                "{}{} — {}",
                "  ".repeat(depth),
                display_name(ctx.w, obj),
                describe_parent(ctx, &s.parent_of(obj))
            ));
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
    let rhs = rhs_required(
        ctx,
        "@spatial/place/domain <obj>=<domain>[,{pose}] | /place/interior <obj>=<space>[,<slot>] | /place/attached <obj>=<host>,<mount>[,{pose}]",
    )?;
    let (parts, json) = split_rhs(&rhs);
    let json = json.map_err(CmdError::msg)?;
    let parent = if ctx.switches.has("domain") {
        let d = resolve_domain(ctx, parts.first().map(String::as_str).unwrap_or(""))?;
        SpatialParent::Direct {
            domain: d,
            pose: pose_from(json)?,
        }
    } else if ctx.switches.has("interior") {
        let space = match_object(
            ctx.w,
            ctx.actor,
            parts.first().map(String::as_str).unwrap_or(""),
        )?;
        let slot = parts.get(1).and_then(|s| SlotId::new(s));
        SpatialParent::Interior { space, slot }
    } else if ctx.switches.has("attached") {
        let host = match_object(
            ctx.w,
            ctx.actor,
            parts.first().map(String::as_str).unwrap_or(""),
        )?;
        let mount = parts.get(1).and_then(|m| MountId::new(m)).ok_or_else(|| {
            CmdError::msg("Which mount? @spatial/place/attached <obj>=<host>,<mount>")
        })?;
        SpatialParent::Attached {
            host,
            mount,
            local_pose: json.map(|j| pose_from(Some(j))).transpose()?,
        }
    } else {
        return Err(CmdError::msg(
            "Say how: @spatial/place/domain, /place/interior, or /place/attached.",
        ));
    };
    let desc = describe_parent(ctx, &parent);
    let control_generation = prepare_control_bump_if_host(ctx, obj)?;
    run(ctx, &Transition::Place { obj, parent }).map_err(sim_err)?;
    // A (re-)placed unit grants control to its crew again.
    refresh_unit_actors(ctx.w, obj);
    apply_control_bump(ctx, obj, control_generation);
    info(ctx, format!("Placed {} {desc}.", display_name(ctx.w, obj)));
    Ok(())
}

fn spatial_unplace(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let obj = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let reason = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("staff unplace")
        .to_string();
    let keep = ctx.switches.has("keep");
    let control_generation = prepare_control_bump_if_host(ctx, obj)?;
    run(
        ctx,
        &Transition::Unplace {
            obj,
            reason,
            by: Some(ctx.actor),
            keep,
        },
    )
    .map_err(sim_err)?;
    // An unplaced unit stops granting control to its crew.
    refresh_unit_actors(ctx.w, obj);
    apply_control_bump(ctx, obj, control_generation);
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
        if s.as_bytes()
            .get(..pfx.len())
            .is_some_and(|h| h.eq_ignore_ascii_case(pfx.as_bytes()))
        {
            let rest = s[pfx.len()..].trim();
            return if fixed {
                Ok(SpaceAnchor::FixedDomain {
                    domain: resolve_domain(ctx, rest)?,
                    anchor_pose: None,
                })
            } else {
                Ok(SpaceAnchor::MovingObject {
                    host: match_object(ctx.w, ctx.actor, rest)?,
                })
            };
        }
    }
    Err(CmdError::msg(
        "Anchor: fixed:<domain>, moving:<host>, or unanchored.",
    ))
}

fn spatial_space(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let space = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let anchor = parse_anchor(
        ctx,
        &rhs_required(
            ctx,
            "@spatial/space <room>=fixed:<domain>|moving:<host>|unanchored",
        )?,
    )?;
    if ctx
        .w
        .get(space)
        .is_none_or(|o| o.kind != ObjKind::Room || o.flags.contains(Flags::GOING))
    {
        return Err(CmdError::msg(
            "An interior space must be a live room (cut 1).",
        ));
    }
    if !ctx
        .w
        .has_behavior(space, &mush_core::BehaviorId::new("sim.interior").unwrap())
    {
        return Err(CmdError::msg(
            "The room must carry `sim.interior` first (@behavior/add).",
        ));
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
            if existed {
                "re-anchored"
            } else {
                "is now an interior space"
            }
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
            return Err(CmdError::msg(format!(
                "It still holds {n} occupant(s); move them first."
            )));
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
    info(
        ctx,
        format!(
            "{} is no longer an interior space.",
            display_name(ctx.w, space)
        ),
    );
    Ok(())
}

fn spatial_slot(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let space = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let rhs = rhs_required(ctx, "@spatial/slot <space>=<slot>[,<max>]")?;
    let (parts, _) = split_rhs(&rhs);
    let slot = parts
        .first()
        .and_then(|s| SlotId::new(s))
        .ok_or_else(|| CmdError::msg("Which slot?"))?;
    let max = parts
        .get(1)
        .map(|m| {
            m.parse::<u32>()
                .map_err(|_| CmdError::msg("Max must be a number."))
        })
        .transpose()?;
    let rec = {
        let s = st_mut(ctx);
        let sp = s
            .spaces
            .get_mut(&space)
            .ok_or_else(|| CmdError::msg("That is not an interior space."))?;
        sp.compartments.insert(
            slot.clone(),
            CapacitySpec {
                maximum_items: max,
                ..Default::default()
            },
        );
        sp.clone()
    };
    persist_space(ctx.out, &rec);
    info(
        ctx,
        format!(
            "Compartment `{slot}` set on {}.",
            display_name(ctx.w, space)
        ),
    );
    Ok(())
}

fn spatial_mount(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let host = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let rhs = rhs_required(ctx, "@spatial/mount <host>=<mount>[,<max>][,exposed]")?;
    let (parts, _) = split_rhs(&rhs);
    let mount = parts
        .first()
        .and_then(|m| MountId::new(m))
        .ok_or_else(|| CmdError::msg("Which mount?"))?;
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
    if ctx
        .w
        .get(host)
        .is_none_or(|o| o.flags.contains(Flags::GOING))
    {
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
        let m = s.mounts.entry(host).or_insert_with(|| HostMounts {
            host,
            mounts: Default::default(),
        });
        m.mounts.insert(
            mount.clone(),
            MountSpec {
                capacity: CapacitySpec {
                    maximum_items: max,
                    ..Default::default()
                },
                exposed,
            },
        );
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
    info(
        ctx,
        format!("Mount `{mount}` set on {}.", display_name(ctx.w, host)),
    );
    Ok(())
}

fn spatial_unmount(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let host = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let mount = MountId::new(&rhs_required(ctx, "@spatial/unmount <host>=<mount>")?)
        .ok_or_else(|| CmdError::msg("Which mount?"))?;
    if st(ctx).attachments(host, &mount) > 0 {
        return Err(CmdError::msg("Detach what is mounted there first."));
    }
    let rec = {
        let s = st_mut(ctx);
        let m = s
            .mounts
            .get_mut(&host)
            .ok_or_else(|| CmdError::msg("No such mount."))?;
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
    info(
        ctx,
        format!(
            "Mount `{mount}` removed from {}.",
            display_name(ctx.w, host)
        ),
    );
    Ok(())
}

fn spatial_quarantine_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let mut lines: Vec<String> = st(ctx)
        .placements
        .values()
        .filter(|p| p.unplaced.as_ref().is_some_and(|u| u.quarantined))
        .map(|p| {
            format!(
                "  {} — {}",
                display_name(ctx.w, p.object),
                p.unplaced.as_ref().map_or("", |u| u.reason.as_str())
            )
        })
        .collect();
    lines.extend(st(ctx).held_placements.iter().map(|(object, held)| {
        format!(
            "  {} — HELD unreadable {} ({})",
            display_name(ctx.w, *object),
            held.key,
            held.reason
        )
    }));
    lines.extend(
        st(ctx)
            .unbound_placement_docs
            .iter()
            .map(|(key, reason)| format!("  {key} — HELD unbound ({reason})")),
    );
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
    if let Some(held) = st(ctx).held_placement(obj) {
        return Err(CmdError::msg(format!(
            "{} is held because {} is unreadable; repair or migrate the authoritative record before recovery.",
            display_name(ctx.w, obj),
            held.key
        )));
    }
    let rec = {
        let s = st_mut(ctx);
        let p = s
            .placements
            .get_mut(&obj)
            .filter(|p| p.unplaced.as_ref().is_some_and(|u| u.quarantined))
            .ok_or_else(|| CmdError::msg("That is not quarantined."))?;
        let revision = checked_placement_revision(obj, p.revision).map_err(counter_err)?;
        if let Some(u) = p.unplaced.as_mut() {
            u.quarantined = false;
            u.reason = format!("recovered by staff (was: {})", u.reason);
        }
        p.revision = revision;
        p.clone()
    };
    crate::data::persist_placement(ctx.out, &rec);
    info(
        ctx,
        format!(
            "{} recovered; it is unplaced and may be re-placed.",
            display_name(ctx.w, obj)
        ),
    );
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
            info(
                ctx,
                format!(
                    "Projection drift: {} should be in {} but is in {}.",
                    display_name(ctx.w, obj),
                    display_name(ctx.w, expected),
                    actual.map_or("nowhere".to_string(), |a| display_name(ctx.w, a))
                ),
            );
        }
    }
    let s = st(ctx);
    let q = s
        .placements
        .values()
        .filter(|p| p.unplaced.as_ref().is_some_and(|u| u.quarantined))
        .count()
        + s.held_placements.len()
        + s.unbound_placement_docs.len();
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
    let rhs = rhs_required(
        ctx,
        "@spatial/station <host>=<station>,<role>[,space:<room>]",
    )?;
    let (parts, _) = split_rhs(&rhs);
    let station = parts
        .first()
        .and_then(|s| StationId::new(s))
        .ok_or_else(|| CmdError::msg("Which station?"))?;
    let role = parts
        .get(1)
        .cloned()
        .filter(|r| !r.is_empty())
        .ok_or_else(|| CmdError::msg("Give a role (pilot, gunner, ...)."))?;
    let mut space: Option<ObjId> = None;
    let mut channels: BTreeSet<ChannelId> = BTreeSet::new();
    let mut priority: Option<u16> = None;
    for part in parts.iter().skip(2) {
        if let Some(room) = part.strip_prefix("space:") {
            space = Some(match_object(ctx.w, ctx.actor, room.trim())?);
        } else if let Some(pr) = part.strip_prefix("priority:") {
            priority = Some(
                pr.trim()
                    .parse()
                    .map_err(|_| CmdError::msg("priority: must be a number 0-65535."))?,
            );
        } else if let Some(cs) = part.strip_prefix("channels:") {
            for c in cs
                .split(['+', ','])
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                channels.insert(
                    ChannelId::new(c)
                        .ok_or_else(|| CmdError::msg(format!("Bad channel `{c}`.")))?,
                );
            }
        } else {
            return Err(CmdError::msg(format!("Unknown station option `{part}`.")));
        }
    }
    let priority = priority.unwrap_or_else(|| default_station_priority(&role));
    let policy = pose_from(None)?;
    if ctx
        .w
        .get(host)
        .is_none_or(|o| o.flags.contains(Flags::GOING))
    {
        return Err(CmdError::msg("No such host."));
    }
    let control_generation = st(ctx).next_control_generation(host).map_err(counter_err)?;
    let rec = {
        let s = st_mut(ctx);
        let hs = s.stations.entry(host).or_insert_with(|| HostStations {
            host,
            stations: Default::default(),
        });
        hs.stations.insert(
            station.clone(),
            StationSpec {
                role: SmolStr::new(&role),
                channels: channels.clone(),
                priority,
                space,
                policy,
            },
        );
        let rec = hs.clone();
        s.apply_control_generation(host, control_generation);
        rec
    };
    persist_stations(ctx.out, &rec);
    persist_control_gen(ctx.out, host, control_generation);
    // A redefined station changes priority/channels; refresh any assigned crew.
    refresh_unit_actors(ctx.w, host);
    // Non-fatal authoring lint: a granted channel no command requires is likely a
    // typo — but stay quiet until channelled commands exist.
    let required = ctx.reg.required_channels();
    if !required.is_empty() {
        let unknown: Vec<String> = channels
            .iter()
            .filter(|c| !required.contains(*c))
            .map(ToString::to_string)
            .collect();
        if !unknown.is_empty() {
            info(
                ctx,
                format!(
                    "Note: no registered command requires channel(s): {}.",
                    unknown.join(", ")
                ),
            );
        }
    }
    info(
        ctx,
        format!(
            "Station `{station}` ({role}, priority {priority}) set on {}.",
            display_name(ctx.w, host)
        ),
    );
    Ok(())
}

fn spatial_unstation(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let host = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let station = StationId::new(&rhs_required(ctx, "@spatial/unstation <host>=<station>")?)
        .ok_or_else(|| CmdError::msg("Which station?"))?;
    if st(ctx)
        .assignments
        .get(&host)
        .is_some_and(|a| a.seats.contains_key(&station))
    {
        return Err(CmdError::msg(
            "Someone holds that station; @spatial/unassign first.",
        ));
    }
    let rec = {
        let s = st_mut(ctx);
        let hs = s
            .stations
            .get_mut(&host)
            .ok_or_else(|| CmdError::msg("No such station."))?;
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
    info(
        ctx,
        format!(
            "Station `{station}` removed from {}.",
            display_name(ctx.w, host)
        ),
    );
    Ok(())
}

fn spatial_assign(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let host = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let rhs = rhs_required(ctx, "@spatial/assign <host>=<station>,<actor>")?;
    let (parts, _) = split_rhs(&rhs);
    let station = parts
        .first()
        .and_then(|s| StationId::new(s))
        .ok_or_else(|| CmdError::msg("Which station?"))?;
    let actor = match_object(
        ctx.w,
        ctx.actor,
        parts.get(1).map(String::as_str).unwrap_or(""),
    )?;
    if ctx
        .w
        .get(actor)
        .is_none_or(|o| o.flags.contains(Flags::GOING))
    {
        return Err(CmdError::msg("No such actor."));
    }
    if !st(ctx)
        .stations
        .get(&host)
        .is_some_and(|hs| hs.stations.contains_key(&station))
    {
        return Err(CmdError::msg(
            "That host has no such station (@spatial/station first).",
        ));
    }
    if let Some(a) = st(ctx).assignments.get(&host)
        && a.seats
            .get(&station)
            .is_some_and(|seat| seat.actor != actor)
    {
        return Err(CmdError::msg(
            "That station is already held; @spatial/unassign first.",
        ));
    }
    let control_generation = st(ctx).next_control_generation(host).map_err(counter_err)?;
    let now = ctx.w.now();
    {
        let s = st_mut(ctx);
        let ha = s
            .assignments
            .entry(host)
            .or_insert_with(|| HostAssignments {
                host,
                seats: Default::default(),
            });
        ha.seats.insert(
            station.clone(),
            StationAssignment {
                actor,
                assigned_at: now,
                // E7c10 B1: staff surgery seats the WHOLE station (`None` = the
                // station's own channel set). Only a proxy seat narrows, and
                // only through `seat::request_seat_activation`.
                granted: None,
            },
        );
        let rec = ha.clone();
        s.derived = crate::graph::rebuild(s);
        s.apply_control_generation(host, control_generation);
        persist_assignments(ctx.out, &rec);
        persist_control_gen(ctx.out, host, control_generation);
    }
    refresh_actor_links(ctx.w, actor);
    info(
        ctx,
        format!(
            "{} now holds `{station}` on {}.",
            display_name(ctx.w, actor),
            display_name(ctx.w, host)
        ),
    );
    Ok(())
}

fn spatial_unassign(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let host = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let station = StationId::new(&rhs_required(ctx, "@spatial/unassign <host>=<station>")?)
        .ok_or_else(|| CmdError::msg("Which station?"))?;
    let former = st(ctx)
        .assignments
        .get(&host)
        .ok_or_else(|| CmdError::msg("No assignments on that host."))?
        .seats
        .get(&station)
        .ok_or_else(|| CmdError::msg("No one holds that station."))?
        .actor;
    let control_generation = st(ctx).next_control_generation(host).map_err(counter_err)?;
    let former = {
        let s = st_mut(ctx);
        let ha = s
            .assignments
            .get_mut(&host)
            .expect("assignment existed during preflight");
        let asg = ha
            .seats
            .remove(&station)
            .expect("station assignment existed during preflight");
        debug_assert_eq!(asg.actor, former);
        let rec = ha.clone();
        if rec.seats.is_empty() {
            s.assignments.remove(&host);
        }
        s.derived = crate::graph::rebuild(s);
        s.apply_control_generation(host, control_generation);
        persist_assignments(ctx.out, &rec);
        persist_control_gen(ctx.out, host, control_generation);
        asg.actor
    };
    refresh_actor_links(ctx.w, former);
    info(
        ctx,
        format!(
            "{} no longer holds `{station}` on {}.",
            display_name(ctx.w, former),
            display_name(ctx.w, host)
        ),
    );
    Ok(())
}

/// `@spatial/authority <actor>` — the read-only inspector: what the actor's
/// derived station grants (subject, station, relation, priority, channels,
/// epoch). Diagnoses fail-safe-open channel typos and stale queued intents.
fn spatial_authority(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let lines: Vec<String> = {
        ctx.w
            .command_subjects_of(actor)
            .iter()
            .map(|l| {
                let ch: Vec<&str> = l
                    .channels
                    .iter()
                    .map(mush_core::ChannelId::as_str)
                    .collect();
                let chans = if ch.is_empty() {
                    "none".to_string()
                } else {
                    ch.join("+")
                };
                format!(
                    "  {} — {}:{} (priority {}, channels: {}, epoch {})",
                    display_name(ctx.w, l.subject),
                    l.station,
                    l.relation,
                    l.priority,
                    chans,
                    l.epoch,
                )
            })
            .collect()
    };
    if lines.is_empty() {
        info(
            ctx,
            format!("{} operates nothing.", display_name(ctx.w, actor)),
        );
    } else {
        info(ctx, format!("{} operates:", display_name(ctx.w, actor)));
        for l in lines {
            info(ctx, l);
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------
// E7c5: focus, billets, identity (cut 5)
// ----------------------------------------------------------------------

fn focus_state<'a>(ctx: &'a Ctx<'_>) -> &'a FocusState {
    ctx.w
        .module_state::<FocusState>()
        .expect("focus state slot installed at registration")
}

fn account_oracle<'a>(ctx: &'a Ctx<'_>) -> &'a AccountOracle {
    ctx.w
        .module_state::<AccountOracle>()
        .expect("account oracle slot installed at registration")
}

/// The `(unit, station)` an actor currently drives in the foreground, if any —
/// the single predicate the command path consults (B3). A billet **is**
/// `assignments.seats[station].actor == actor` plus `stations.stations[station]`;
/// this reports which one is focused.
#[must_use]
pub(crate) fn focused_station(ctx: &Ctx<'_>, actor: ObjId) -> Option<(ObjId, StationId)> {
    focused_station_of(ctx.w, actor)
}

/// The same predicate, over a bare `&World`.
///
/// **E7 cut 8 independent verify fold:** `TacticalIntentProvider::project_offer`
/// has an `&Engine` and no `Ctx`, and `DomainTickScheduleAdapter::plan` needs
/// the same answer to run A7's focus companion beside the grant witness. There
/// is still exactly ONE focus predicate; `focused_station` delegates to it.
#[must_use]
pub(crate) fn focused_station_of(
    world: &mush_core::World,
    actor: ObjId,
) -> Option<(ObjId, StationId)> {
    world
        .module_state::<FocusState>()?
        .focus_of_actor(actor)
        .map(|f| (f.unit, f.station.clone()))
}

/// Whether `actor` holds the seat `station` on `unit` (a billet), and the unit
/// actually defines that station. No focus implied — billet is eligibility.
pub(crate) fn holds_billet(ctx: &Ctx<'_>, actor: ObjId, unit: ObjId, station: &StationId) -> bool {
    let s = st(ctx);
    s.stations
        .get(&unit)
        .is_some_and(|hs| hs.stations.contains_key(station))
        && s.assignments
            .get(&unit)
            .and_then(|ha| ha.seats.get(station))
            .is_some_and(|seat| seat.actor == actor)
}

/// The console room a station is operated from: its backing cockpit `space` if
/// authored, else the unit's own projected location. `None` when neither exists
/// (reach then fails closed).
pub(crate) fn console_room(ctx: &Ctx<'_>, unit: ObjId, station: &StationId) -> Option<ObjId> {
    let s = st(ctx);
    let space = s
        .stations
        .get(&unit)
        .and_then(|hs| hs.stations.get(station))
        .and_then(|spec| spec.space);
    space.or_else(|| ctx.w.get(unit).and_then(|o| o.location))
}

/// Preflight one control-epoch bump for a unit (fails closed on exhaustion,
/// **before** any mutation — invariant: a claim whose bump fails leaves no
/// focus).
pub(crate) fn prepare_focus_bump(ctx: &Ctx<'_>, unit: ObjId) -> Result<u64, CmdError> {
    st(ctx).next_control_generation(unit).map_err(counter_err)
}

/// Commit a preflighted bump and republish the actor's links — **bump then
/// republish**, in that order (contradiction (v)), so the republished grant
/// carries the new epoch and every stale intent fails `channel_grant_is_current`.
pub(crate) fn apply_focus_bump_and_refresh(
    ctx: &mut Ctx<'_>,
    unit: ObjId,
    next: u64,
    actor: ObjId,
) {
    st_mut(ctx).apply_control_generation(unit, next);
    persist_control_gen(ctx.out, unit, next);
    refresh_actor_links(ctx.w, actor);
}

fn cmd_focus(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("release") {
        focus_release(ctx)
    } else if ctx.args.raw().trim().is_empty() {
        focus_show(ctx)
    } else {
        focus_acquire(ctx)
    }
}

/// `+focus <unit>/<station>` — take foreground control of one billet, atomically
/// relinquishing the account's prior focus (invariant 1).
fn focus_acquire(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_string();
    let (unit_ref, station_ref) = raw
        .split_once('/')
        .ok_or_else(|| CmdError::msg("Usage: +focus <unit>/<station>"))?;
    let unit = match_object(ctx.w, ctx.actor, unit_ref.trim())?;
    let station = StationId::new(station_ref)
        .ok_or_else(|| CmdError::msg("Usage: +focus <unit>/<station>"))?;
    // The immutable account. `None` = unknown = refuse (never a default).
    let account = account_oracle(ctx).account_of(ctx.actor).ok_or_else(|| {
        CmdError::msg(
            "Your account is not mapped yet; focus is unavailable until you reconnect (staff: relink the character).",
        )
    })?;
    // Ownership alone never satisfies control (invariant 5): a billet is required.
    if !holds_billet(ctx, ctx.actor, unit, &station) {
        return Err(CmdError::msg(format!(
            "You hold no billet `{station}` on {}.",
            display_name(ctx.w, unit)
        )));
    }
    // Reach: a console must be co-located (cut 5's stub; cut 7 tightens it).
    let room = console_room(ctx, unit, &station).ok_or_else(|| {
        CmdError::msg("That station has no console you can reach (it is nowhere placed).")
    })?;
    let reach = ReachProof::Console { room };
    if !reach.is_reachable(ctx.w, ctx.actor) {
        return Err(CmdError::msg(format!(
            "You are not at {}'s console; move there first.",
            display_name(ctx.w, unit)
        )));
    }
    // Preflight every bump BEFORE touching focus. The prior focus (if any, on a
    // different unit) is released; both witness bumps are checked here so an
    // exhausted epoch refuses with zero mutation and no focus is left behind.
    let prior_unit = focus_state(ctx).focus_of_account(account).map(|f| f.unit);
    let new_gen = prepare_focus_bump(ctx, unit)?;
    let prior_gen = match prior_unit {
        Some(prior) if prior != unit => Some((prior, prepare_focus_bump(ctx, prior)?)),
        _ => None,
    };
    let acquired_at = ctx.w.world_ic_now();
    // Apply — no fallible work past here. Bump the acquired unit, install the
    // focus (relinquishing the prior atomically), then refresh witnesses; a
    // grant on the released unit is republished under its new epoch too.
    apply_focus_bump_and_refresh(ctx, unit, new_gen, ctx.actor);
    let focus = ActiveFocus {
        account,
        actor: ctx.actor,
        unit,
        station: station.clone(),
        acquired_at,
        epoch: new_gen,
        reach,
        // `+focus` is always the actor's OWN billet: never a proxy seat.
        proxy: false,
    };
    let released = ctx
        .w
        .module_state_mut::<FocusState>()
        .expect("focus state slot installed at registration")
        .claim(focus);
    if let Some((prior, prior_generation)) = prior_gen {
        apply_focus_bump_and_refresh(ctx, prior, prior_generation, ctx.actor);
    }
    for prior in &released {
        // A different actor of the same account that was displaced: re-derive
        // its links so its (now non-foreground) grants carry the fresh epoch.
        if prior.actor != ctx.actor {
            refresh_actor_links(ctx.w, prior.actor);
        }
    }
    // C2 / invariant 6: a focus SWITCH releases the prior station just as
    // `+focus/release` does, so it must fire that station's declared fallback too
    // — AFTER the prior unit's epoch bump above, so the fire pins the new grant
    // epoch and its offer re-sources from the surviving billet grant. Mirror
    // `focus_release`: fire for the switching actor's own displaced focus.
    for prior in &released {
        if prior.actor == ctx.actor {
            crate::station::fire_station_fallback(ctx, prior.unit, &prior.station);
        }
    }
    info(
        ctx,
        format!(
            "You take foreground control of `{station}` on {}.",
            display_name(ctx.w, unit)
        ),
    );
    Ok(())
}

/// `+focus/release` — relinquish the account's foreground focus.
fn focus_release(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let account = account_oracle(ctx)
        .account_of(ctx.actor)
        .ok_or_else(|| CmdError::msg("Your account is not mapped; there is nothing to release."))?;
    let Some((unit, station)) = focus_state(ctx)
        .focus_of_account(account)
        .map(|f| (f.unit, f.station.clone()))
    else {
        return Err(CmdError::msg("You hold no foreground focus."));
    };
    let generation = prepare_focus_bump(ctx, unit)?;
    let released = ctx
        .w
        .module_state_mut::<FocusState>()
        .expect("focus state slot installed at registration")
        .relinquish(account);
    // Bump then republish: the released billet still projects its grant (a
    // delayed intent is revalidated and refused), now under the new epoch.
    apply_focus_bump_and_refresh(ctx, unit, generation, ctx.actor);
    if let Some(focus) = released
        && focus.actor != ctx.actor
    {
        refresh_actor_links(ctx.w, focus.actor);
    }
    // B6 / invariant 6 (C1): a clean release fires the released station's declared
    // fallback immediately (the K7.0 immediate-controller seam) — AFTER the epoch
    // bump and republish, so the deferred submit pins the NEW grant epoch and its
    // offer re-sources from the surviving billet grant rather than the focus this
    // release just relinquished. Fired past every fallible step, so no partial
    // release ever queues an orphan submit.
    crate::station::fire_station_fallback(ctx, unit, &station);
    info(ctx, "You relinquish foreground control.");
    Ok(())
}

/// `+focus` (no arg) — show the account's current focus.
fn focus_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(account) = account_oracle(ctx).account_of(ctx.actor) else {
        info(ctx, "Your account is not mapped; you hold no focus.");
        return Ok(());
    };
    let line = focus_state(ctx).focus_of_account(account).map(|f| {
        format!(
            "You control `{}` on {} (epoch {}).",
            f.station,
            display_name(ctx.w, f.unit),
            f.epoch
        )
    });
    match line {
        Some(l) => info(ctx, l),
        None => info(ctx, "You hold no foreground focus."),
    }
    Ok(())
}

/// `+stations` — the actor's own billets, with reach and focusability, bounded
/// on records **visited** (D-LBK-5) and disclosing nothing about a unit's
/// location the viewer may not perceive.
fn cmd_stations(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let focused = focused_station(ctx, actor);
    let mut lines: Vec<String> = Vec::new();
    let mut truncated = false;
    {
        let s = st(ctx);
        // B3: iterate the actor's OWN seats via `seats_by_actor`, so the scan cost
        // is bounded by their own billet count and the `MAX_BILLETS_LISTED` cap —
        // never a walk of every unit's whole (uncapped) `seats` map to skip other
        // actors' seats.
        let own: Vec<(ObjId, StationId)> = s
            .derived
            .seats_by_actor
            .get(&actor)
            .map(|seats| seats.iter().cloned().collect())
            .unwrap_or_default();
        for (unit, station) in own {
            if lines.len() >= MAX_BILLETS_LISTED {
                truncated = true;
                break;
            }
            let role = s
                .stations
                .get(&unit)
                .and_then(|hs| hs.stations.get(&station))
                .map_or_else(|| "?".to_string(), |spec| spec.role.to_string());
            let mark = if focused.as_ref() == Some(&(unit, station.clone())) {
                " [FOCUSED]"
            } else {
                ""
            };
            // Name the unit (the crew member knows their own billet) but never its
            // location.
            lines.push(format!(
                "  {} — `{station}` ({role}){mark}",
                display_name(ctx.w, unit)
            ));
        }
    }
    if lines.is_empty() {
        info(ctx, "You hold no billets.");
    } else {
        info(ctx, "Your billets:");
        for l in lines {
            info(ctx, l);
        }
        if truncated {
            info(
                ctx,
                format!("(only the first {MAX_BILLETS_LISTED} billets are shown.)"),
            );
        }
    }
    Ok(())
}

/// `@identity/audit <a>=<b>` — staff: do two characters share an account?
/// Answers same/different and **never an id** (R7); refuses on the unknown.
fn cmd_identity(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx.switches.has("audit") {
        return Err(CmdError::msg("Usage: @identity/audit <a>=<b>"));
    }
    require_admin(ctx)?;
    let a = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let b = match_object(
        ctx.w,
        ctx.actor,
        &rhs_required(ctx, "@identity/audit <a>=<b>")?,
    )?;
    let verdict = account_oracle(ctx).same_account(a, b);
    let msg = match verdict {
        Some(true) => format!(
            "{} and {} are the SAME account.",
            display_name(ctx.w, a),
            display_name(ctx.w, b)
        ),
        Some(false) => format!(
            "{} and {} are DIFFERENT accounts.",
            display_name(ctx.w, a),
            display_name(ctx.w, b)
        ),
        None => format!(
            "Cannot determine: {} or {} is not mapped to an account.",
            display_name(ctx.w, a),
            display_name(ctx.w, b)
        ),
    };
    info(ctx, msg);
    Ok(())
}

// ----------------------------------------------------------------------
// egress / detach (cut 2) -- Place with a computed parent
// ----------------------------------------------------------------------

/// The Direct parent an occupant egresses to: the space's effective domain
/// (read from the OCCUPANT while still inside) plus the chosen egress pose.
fn egress_parent(
    ctx: &Ctx<'_>,
    unit: ObjId,
    egress_id: Option<&str>,
) -> Result<SpatialParent, CmdError> {
    let s = st(ctx);
    let SpatialParent::Interior { space, .. } = s.parent_of(unit) else {
        return Err(CmdError::msg(format!(
            "{} is not inside anything.",
            display_name(ctx.w, unit)
        )));
    };
    let domain = effective_domain(s, unit).ok_or_else(|| {
        CmdError::msg("That space has no domain to egress onto (it is unanchored).")
    })?;
    let rec = s.spaces.get(&space);
    let pose = match egress_id {
        Some(id) => Some(
            rec.and_then(|r| r.egresses.get(id))
                .ok_or_else(|| CmdError::msg(format!("No egress `{id}` on that space.")))?
                .to_pose
                .clone(),
        ),
        None => match rec.map(|r| &r.egresses) {
            Some(e) if e.len() == 1 => Some(e.values().next().unwrap().to_pose.clone()),
            Some(e) if e.len() > 1 => {
                return Err(CmdError::msg("Which egress? Several are defined."));
            }
            _ => None,
        },
    };
    let pose = match pose.flatten() {
        Some(pose) => pose,
        None => pose_from(None)?,
    };
    Ok(SpatialParent::Direct { domain, pose })
}

fn spatial_egress(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let unit = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let egress_id = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
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
        return Err(CmdError::msg(format!(
            "{} is not attached to anything.",
            display_name(ctx.w, child)
        )));
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

// ----------------------------------------------------------------------
// Journeys — @route / +intercept (mobility, route.rs)
// ----------------------------------------------------------------------

fn rs_ref<'a>(ctx: &'a Ctx<'_>) -> &'a RouteState {
    ctx.w
        .module_state::<RouteState>()
        .expect("route state slot installed at registration")
}
fn rs_mut<'a>(ctx: &'a mut Ctx<'_>) -> &'a mut RouteState {
    ctx.w
        .module_state_mut::<RouteState>()
        .expect("route state slot installed at registration")
}

/// Why a would-be traveler is not eligible for a route (it must be a plain,
/// un-placed object — a traveler is EITHER on a journey OR sim-placed).
fn route_ineligible(ctx: &Ctx<'_>, obj: ObjId) -> Option<String> {
    let name = display_name(ctx.w, obj);
    match ctx.w.get(obj) {
        None => return Some("No such object.".into()),
        Some(o) if o.flags.contains(Flags::GOING) => {
            return Some(format!("{name} is being destroyed."));
        }
        Some(o) if matches!(o.kind, ObjKind::Room | ObjKind::Exit) => {
            return Some(format!("{name} cannot travel a route."));
        }
        _ => {}
    }
    if rs_ref(ctx).journeys.contains_key(&obj) {
        return Some(format!(
            "{name} is already on a route (@route/abort first)."
        ));
    }
    let st = st(ctx);
    if st.is_placed(obj) || st.is_quarantined(obj) {
        return Some(format!(
            "{name} is placed in the simulation; unplace it before routing it."
        ));
    }
    if st.domain_of_host(obj).is_some()
        || st.spaces.contains_key(&obj)
        || st.mounts.contains_key(&obj)
        || st.stations.contains_key(&obj)
    {
        return Some(format!(
            "{name} is simulation infrastructure and cannot travel a route."
        ));
    }
    None
}

fn cmd_route(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_admin(ctx)?;
    let sw = &ctx.switches;
    if sw.has("show") {
        route_show(ctx)
    } else if sw.has("pause") {
        route_ctl(ctx, true)
    } else if sw.has("resume") {
        route_ctl(ctx, false)
    } else if sw.has("abort") {
        route_abort(ctx)
    } else if ctx.args.lhs().trim().is_empty() {
        route_list(ctx)
    } else {
        route_create(ctx)
    }
}

fn route_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let traveler = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    if let Some(why) = route_ineligible(ctx, traveler) {
        return Err(CmdError::msg(why));
    }
    let rhs = rhs_required(ctx, "@route <traveler>=<room>[:secs] <room>[:secs] ...")?;
    let mut legs: Vec<Leg> = Vec::new();
    for tok in rhs.split_whitespace() {
        let (room_ref, secs) = match tok.rsplit_once(':') {
            Some((r, n)) if n.parse::<u32>().is_ok() => (r, n.parse::<u32>().unwrap()),
            _ => (tok, route::DEFAULT_TRAVEL_S),
        };
        if secs < 1 {
            return Err(CmdError::msg("A leg must take at least 1 second."));
        }
        let room = match_object(ctx.w, ctx.actor, room_ref)?;
        if ctx
            .w
            .get(room)
            .is_none_or(|o| o.kind != ObjKind::Room || o.flags.contains(Flags::GOING))
        {
            return Err(CmdError::msg(format!("`{room_ref}` is not a live room.")));
        }
        // Authoring-time enterability check (the sweep bypasses gates later).
        ctx.w.can_move_to(traveler, room).map_err(|e| {
            CmdError::msg(format!(
                "cannot route through {}: {e}",
                display_name(ctx.w, room)
            ))
        })?;
        legs.push(Leg {
            to: room,
            travel_s: secs,
        });
    }
    if legs.is_empty() {
        return Err(CmdError::msg("A route needs at least one room."));
    }
    let now = ctx.w.now();
    let destination = legs.last().unwrap().to;
    let j = Journey {
        traveler,
        legs,
        cursor: 0,
        started_at: now,
        status: JourneyStatus::Traveling,
        paused_at: None,
        grace_until: 0,
        destination,
        by: ctx.actor,
    };
    route::persist_journey(ctx.out, &j);
    let n = j.legs.len();
    rs_mut(ctx).journeys.insert(traveler, j);
    info(
        ctx,
        format!(
            "{} sets out on a {}-leg route to {}.",
            display_name(ctx.w, traveler),
            n,
            display_name(ctx.w, destination)
        ),
    );
    Ok(())
}

fn route_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let traveler = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let lines: Vec<String> = {
        let rs = rs_ref(ctx);
        let j = rs
            .journeys
            .get(&traveler)
            .ok_or_else(|| CmdError::msg("It is not on a route."))?;
        let mut v = vec![format!(
            "{}: {:?}, leg {}/{}, heading for {}.",
            display_name(ctx.w, traveler),
            j.status,
            j.cursor,
            j.legs.len(),
            display_name(ctx.w, j.destination)
        )];
        for (i, leg) in j.legs.iter().enumerate() {
            let mark = if i < j.cursor { "*" } else { " " };
            v.push(format!(
                "  {mark} {} ({}s)",
                display_name(ctx.w, leg.to),
                leg.travel_s
            ));
        }
        if let Some(nd) = j.next_due() {
            v.push(format!("  next hop due at {} (now {}).", nd, ctx.w.now()));
        }
        v
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

fn route_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lines: Vec<String> = rs_ref(ctx)
        .journeys
        .values()
        .map(|j| {
            format!(
                "  {} — {:?}, leg {}/{} to {}",
                display_name(ctx.w, j.traveler),
                j.status,
                j.cursor,
                j.legs.len(),
                display_name(ctx.w, j.destination)
            )
        })
        .collect();
    if lines.is_empty() {
        info(ctx, "No active routes.");
    } else {
        info(ctx, "Routes:");
        for l in lines {
            info(ctx, l);
        }
    }
    Ok(())
}

fn route_ctl(ctx: &mut Ctx<'_>, pausing: bool) -> Result<(), CmdError> {
    let traveler = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    let now = ctx.w.now();
    let rec = {
        let rs = rs_mut(ctx);
        if pausing {
            route::pause(rs, traveler, now, false)
        } else {
            route::resume(rs, traveler, now)
        }
        .map_err(CmdError::msg)?;
        rs.journeys.get(&traveler).cloned()
    };
    if let Some(j) = rec {
        route::persist_journey(ctx.out, &j);
    }
    info(
        ctx,
        format!(
            "{} {}.",
            display_name(ctx.w, traveler),
            if pausing {
                "halted"
            } else {
                "resumes its route"
            }
        ),
    );
    Ok(())
}

fn route_abort(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let traveler = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    if rs_mut(ctx).journeys.remove(&traveler).is_none() {
        return Err(CmdError::msg("It is not on a route."));
    }
    route::persist_journey_del(ctx.out, traveler);
    info(
        ctx,
        format!("{}'s route is cancelled.", display_name(ctx.w, traveler)),
    );
    Ok(())
}

/// Player-facing `+intercept` / `waylay`: stop a traveler passing through your
/// room. Requires co-location and perception; anti-grief grace after a resume.
pub(crate) fn intercept_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let traveler = match_object(ctx.w, ctx.actor, ctx.args.raw().trim())?;
    let here = ctx.w.get(ctx.actor).and_then(|o| o.location);
    let there = ctx.w.get(traveler).and_then(|o| o.location);
    if here.is_none() || here != there {
        return Err(CmdError::msg("That is not here."));
    }
    if !ctx.can_perceive(ctx.actor, traveler) {
        return Err(CmdError::msg("You don't see it."));
    }
    let now = ctx.w.now();
    let rec = {
        let rs = rs_mut(ctx);
        route::pause(rs, traveler, now, true).map_err(CmdError::msg)?;
        rs.journeys.get(&traveler).cloned()
    };
    if let Some(j) = &rec {
        route::persist_journey(ctx.out, j);
    }
    let room = here.unwrap();
    let text = mush_proto::RichText::new()
        .span(mush_core::output::name_span(ctx.w, ctx.actor))
        .text(" waylays ")
        .span(mush_core::output::name_span(ctx.w, traveler))
        .text("!");
    // NOTE (epistemic layer): this names `traveler` to the whole room, gated on
    // the interceptor as subject. A concealed traveler's name/presence would
    // leak to occupants who cannot perceive it. When per-observer name
    // projection exists (Obfuscate/Umbra/&c.), this fan-out must project the
    // traveler per observer instead of naming it uniformly. Harmless today —
    // no concealment mechanic renders yet.
    ctx.out.to_room_from(
        room,
        vec![],
        ctx.actor,
        mush_proto::OutputEvent::Narrative {
            kind: mush_proto::NarrativeKind::Emit,
            actor: None,
            text,
        },
    );
    ctx.out.sys_to_actor(
        ctx.actor,
        "sim",
        format!("You stop {}.", display_name(ctx.w, traveler)),
    );
    Ok(())
}

// ----------------------------------------------------------------------
// ---- E7c6: B3/B6 — @journey ----
// ----------------------------------------------------------------------

/// Default seconds for a leg the staff author gives no explicit time.
const DEFAULT_LEG_S: u32 = route::DEFAULT_TRAVEL_S;
/// The drive class a staff-authored corridor is flown by. Cut 6's journeys take
/// their leg boundaries straight from each `LegDef::base_s` — which is exactly
/// what `route_duration` yields at nominal thrust with no payload — because cut
/// 6 has no pack lookup for a `DriveProfile` and inventing one would be a second
/// source of truth for a duration T1 froze.
const STAFF_DRIVE_CLASS: &str = "manual";

/// The one rule-4 line every refused journey read answers with, so a carrier
/// with no journey, a carrier the viewer does not crew and an object that never
/// existed are byte-identical.
const JOURNEY_NONE: &str = "No journey.";

fn jb<'a>(ctx: &'a Ctx<'_>) -> &'a JourneyBook {
    ctx.w
        .module_state::<JourneyBook>()
        .expect("journey book slot installed at registration")
}

/// The object a carrier's current placement hangs on — the route's origin
/// endpoint. A carrier that is nowhere has no journey to plan.
fn origin_anchor(ctx: &Ctx<'_>, carrier: ObjId) -> Result<ObjId, CmdError> {
    let s = st(ctx);
    match s.parent_of(carrier) {
        SpatialParent::Direct { domain, .. } => s
            .domains
            .get(&domain)
            .map(|d| d.host)
            .ok_or_else(|| CmdError::msg("That carrier's domain no longer exists.")),
        SpatialParent::Interior { space, .. } => Ok(space),
        SpatialParent::Attached { host, .. } => Ok(host),
        SpatialParent::Unplaced => Err(CmdError::msg(
            "That carrier is not placed anywhere; place it first (@spatial/place/*).",
        )),
    }
}

/// The placement a carrier comes to rest at when it reaches `place`: a live
/// domain's direct placement, or an interior space.
fn arrival_parent(ctx: &Ctx<'_>, place: ObjId) -> Result<SpatialParent, CmdError> {
    let s = st(ctx);
    if let Some(d) = s.domain_of_host(place) {
        if d.status != DomainStatus::Active {
            return Err(CmdError::msg(format!(
                "Domain #{} is {:?} and accepts no arrivals.",
                d.id, d.status
            )));
        }
        let id = d.id;
        return Ok(SpatialParent::Direct {
            domain: id,
            pose: pose_from(None)?,
        });
    }
    if s.spaces.contains_key(&place) {
        // **A journey may not dock into a custody backing** (E7 cut 6 fold,
        // C7). `InteriorCustodyProvider::contents` derives a vault's stock
        // purely from the placement graph, so an arrival inside a vault-backed
        // interior would silently make a ship, its bays and everyone aboard
        // vault stock — with no economy act, no journal entry and no owner
        // assent — and the departure would remove them just as silently.
        //
        // The read is direct: `mush-sim -> mush-economy` is a pre-existing,
        // unconditional edge (E3c's `CustodyProvider`), so this needs no oracle
        // and no composition parameter. `None` (the economy is not composed)
        // means "not wired", never "no", like every other optional-module read
        // in the crate. Docking into a vault wants a real deposit act, which is
        // a write across that edge and therefore a later cut's.
        if ctx
            .w
            .module_state::<mush_economy::EconomyStore>()
            .is_some_and(|store| store.snapshot().is_vault_backing(place))
        {
            return Err(CmdError::msg(format!(
                "{} backs a custody vault; a journey cannot dock into one (a deposit is an \
                 economy act, not a placement).",
                display_name(ctx.w, place)
            )));
        }
        return Ok(SpatialParent::Interior {
            space: place,
            slot: None,
        });
    }
    Err(CmdError::msg(
        "A journey's destination must host a live domain (@domain/create) or be an interior space \
         (@spatial/space).",
    ))
}

/// Parse `<route id> <place>[:secs] <place>[:secs] …` into the definition the
/// journey is flown from, plus the placement its last leg arrives at.
///
/// **E7c7 (cut 6 fold, B6).** The id is first offered to the
/// composition-installed [`RouteCatalogState`] — an authored `RouteDef` carries
/// its own legs, drive class, declared propellant and alignment band, so those
/// three accessors finally have a caller. A hand-typed id that names no authored
/// route, or a line that also lists places, still builds the **manual fixture
/// route** exactly as cut 6 did: the catalog is a resolution, never a gate.
fn build_route(
    ctx: &Ctx<'_>,
    origin: ObjId,
    spec: &str,
) -> Result<(RouteDef, SpatialParent), CmdError> {
    let mut tokens = spec.split_whitespace();
    let id_token = tokens
        .next()
        .ok_or_else(|| CmdError::msg("Usage: @journey/depart <carrier>=<route id> <place>…"))?;
    if id_token.len() > mush_sim_max_route_id_bytes() {
        return Err(CmdError::msg(format!(
            "A route id is at most {} bytes.",
            mush_sim_max_route_id_bytes()
        )));
    }
    let id = CanonicalId::new(id_token)
        .map_err(|e| CmdError::msg(format!("`{id_token}` is not a usable route id: {e}")))?;
    let mut tokens = tokens.peekable();
    // **The catalog is consulted on the ID FIRST** (E7 cut 7 independent verify
    // fold, O5). Before the fold the authored arm was gated on
    // `tokens.peek().is_none()`, so ONE extra place token skipped it entirely —
    // including its `from != origin` refusal — and fell into the manual parser,
    // which minted `RouteDef::new(id, …, STAFF_DRIVE_CLASS, 0)` **carrying the
    // authored id** from wherever the carrier happened to be, with none of the
    // authored propellant, alignment band or drive class. The persisted journey
    // then recorded an authored route it had not flown.
    if let Some(authored) = ctx
        .w
        .module_state::<crate::route::RouteCatalogState>()
        .and_then(|catalog| catalog.resolve(ctx.w, &id))
    {
        if tokens.peek().is_some() {
            return Err(CmdError::msg(format!(
                "Route `{id_token}` is authored; drop the place list."
            )));
        }
        let (from, to) = authored.endpoints();
        if from != origin {
            // §4 rule 4 (fold O12): a crew member may read a journey, so the
            // informative form is staff-only — otherwise `/plan` is an oracle
            // over which route ids exist and where each begins.
            return Err(CmdError::msg(if is_admin(ctx.w, ctx.actor) {
                format!(
                    "Route `{id_token}` sets out from {}; {} is at {}.",
                    display_name(ctx.w, from),
                    display_name(ctx.w, origin),
                    display_name(ctx.w, origin)
                )
            } else {
                ROUTE_NOT_FROM_HERE.to_owned()
            }));
        }
        let parent = arrival_parent(ctx, to)?;
        return Ok((authored, parent));
    }
    // A bare id the catalog cannot answer for. For staff that is a usage error;
    // for anyone else it is the SAME line an authored route from elsewhere
    // yields, so the two are indistinguishable (fold O12).
    if tokens.peek().is_none() && !is_admin(ctx.w, ctx.actor) {
        return Err(CmdError::msg(ROUTE_NOT_FROM_HERE));
    }
    let mut legs: Vec<LegDef> = Vec::new();
    let mut last: Option<ObjId> = None;
    for token in tokens {
        // A route past MAX_LEGS is refused by `RouteDef::new` below; refuse the
        // parse early too so a thousand-token line never builds a thousand legs.
        if legs.len() >= MAX_LEGS {
            return Err(CmdError::msg(format!(
                "A route declares at most {MAX_LEGS} legs."
            )));
        }
        let (place_ref, secs) = match token.rsplit_once(':') {
            Some((p, n)) => match n.parse::<u32>() {
                Ok(n) => (p, n),
                Err(_) => (token, DEFAULT_LEG_S),
            },
            None => (token, DEFAULT_LEG_S),
        };
        let place = match_object(ctx.w, ctx.actor, place_ref)?;
        if ctx
            .w
            .get(place)
            .is_none_or(|o| o.flags.contains(Flags::GOING))
        {
            return Err(CmdError::msg(format!("`{place_ref}` is not a live place.")));
        }
        legs.push(
            LegDef::new(place, secs)
                .map_err(|e| CmdError::msg(format!("leg `{place_ref}`: {e}")))?,
        );
        last = Some(place);
    }
    let destination = last.ok_or_else(|| CmdError::msg("A journey needs at least one leg."))?;
    let parent = arrival_parent(ctx, destination)?;
    let drive_class = CanonicalId::new(STAFF_DRIVE_CLASS)
        .map_err(|e| CmdError::msg(format!("bad drive class: {e}")))?;
    let route = RouteDef::new(id, (origin, destination), legs, None, drive_class, 0)
        .map_err(|e| CmdError::msg(e.to_string()))?;
    Ok((route, parent))
}

const fn mush_sim_max_route_id_bytes() -> usize {
    crate::route::MAX_ROUTE_ID_BYTES
}

/// The one line a non-staff actor gets for an authored route that does not set
/// out from here — byte-identical to what an unknown id yields, so a crew route
/// probe learns nothing (§4 rule 4, fold O12).
pub const ROUTE_NOT_FROM_HERE: &str = "That route does not set out from here.";

/// **The carrier-root invariant and the seal, surfaced early.**
/// `move_carrier_subtree` refuses a root that would come to rest inside its own
/// subtree or inside ANOTHER carrier that is under way, but refusing either at
/// arrival would strand a carrier that had already flown the whole passage. The
/// same two checks run here, at `/plan`, `/depart` and `/divert`, so an
/// unreachable destination never departs (E7 cut 6 fold, C1(b)).
fn refuse_unreachable_destination(
    ctx: &Ctx<'_>,
    carrier: ObjId,
    destination: &SpatialParent,
) -> Result<(), CmdError> {
    let anchor = match destination {
        SpatialParent::Direct { domain, .. } => st(ctx).domains.get(domain).map(|d| d.host),
        SpatialParent::Interior { space, .. } => Some(*space),
        SpatialParent::Attached { host, .. } => Some(*host),
        SpatialParent::Unplaced => None,
    };
    let Some(anchor) = anchor else { return Ok(()) };
    if anchor == carrier || st(ctx).subtree(carrier).contains(&anchor) {
        return Err(CmdError::msg(format!(
            "{} carries {} — a carrier cannot come to rest inside its own hold, at any depth.",
            display_name(ctx.w, carrier),
            display_name(ctx.w, anchor)
        )));
    }
    if let Some(root) = mush_sim_under_way_root_of(ctx, anchor) {
        return Err(CmdError::msg(format!(
            "{} is aboard {}, which is under way; nothing crosses a carrier's boundary \
             mid-journey, so a journey cannot end there.",
            display_name(ctx.w, anchor),
            display_name(ctx.w, root)
        )));
    }
    Ok(())
}

/// The under-way carrier root `obj` rides, if any — the one predicate the seal
/// uses, read from the command path.
fn mush_sim_under_way_root_of(ctx: &Ctx<'_>, obj: ObjId) -> Option<ObjId> {
    crate::journey::under_way_root_of(st(ctx), jb(ctx), obj)
}

/// Why a carrier may not set out. One predicate, shared by `/plan`, `/depart`
/// and `/divert`, so the dry run and the commit can never disagree.
fn journey_ineligible(ctx: &Ctx<'_>, carrier: ObjId) -> Option<String> {
    let name = display_name(ctx.w, carrier);
    match ctx.w.get(carrier) {
        None => return Some("No such object.".into()),
        Some(o) if o.flags.contains(Flags::GOING) => {
            return Some(format!("{name} is being destroyed."));
        }
        Some(o) if matches!(o.kind, ObjKind::Room | ObjKind::Exit) => {
            return Some(format!("{name} cannot fly a route."));
        }
        _ => {}
    }
    if jb(ctx).under_way(carrier) {
        return Some(format!(
            "{name} is already under way (@journey/divert to re-plan it)."
        ));
    }
    // **The seal, read as a command refusal** (E7 cut 6 fold, C1(a)). Without
    // this, any `sim.participant` inside a cruising carrier's subtree could be
    // given a journey of its own and leave the sealed subtree mid-leg — the
    // exact act `@spatial/place` refuses with `CarrierSealed`.
    if let Some(root) = mush_sim_under_way_root_of(ctx, carrier)
        && root != carrier
    {
        return Some(format!(
            "{name} is aboard {}, which is under way; nothing crosses a carrier's boundary \
             mid-journey.",
            display_name(ctx.w, root)
        ));
    }
    if rs_ref(ctx).is_traveling(carrier) {
        return Some(format!(
            "{name} is on a legacy @route; @route/abort it before giving it a journey."
        ));
    }
    // The census (E7 cut 6 fold, B8): the `cjourney:` key space is bounded, and
    // the bound is checked before the record exists, not after.
    if jb(ctx).journeys.len() >= crate::journey::MAX_CARRIER_JOURNEYS
        && !jb(ctx).journeys.contains_key(&carrier)
    {
        return Some(format!(
            "This world already holds {} carrier journeys ({name} would be one too many); close \
             one before opening another.",
            crate::journey::MAX_CARRIER_JOURNEYS
        ));
    }
    let s = st(ctx);
    if s.held_placement(carrier).is_some() {
        return Some(format!(
            "{name} has a held unreadable placement; staff must repair it first."
        ));
    }
    if s.is_quarantined(carrier) {
        return Some(format!("{name} is quarantined; @spatial/recover it first."));
    }
    None
}

/// Whether `actor` crews `unit` at ANY of its authored stations — the read
/// gate for `@journey/show|list|plan` (E7 cut 6 fold, D5(b)). Reuses cut 5's
/// `holds_billet` exactly, so "who may read this ship's journey" and "who may
/// drive it" have one answer.
fn crews(ctx: &Ctx<'_>, actor: ObjId, unit: ObjId) -> bool {
    st(ctx).stations.get(&unit).is_some_and(|hs| {
        hs.stations
            .keys()
            .any(|s| holds_billet(ctx, actor, unit, s))
    })
}

/// The read gate: staff, or an actor who holds a billet on that carrier.
fn may_read_journey(ctx: &Ctx<'_>, carrier: ObjId) -> bool {
    is_admin(ctx.w, ctx.actor) || crews(ctx, ctx.actor, carrier)
}

fn cmd_journey(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // **The write half stays staff-only for cut 6** (D5(b)): `/depart` and
    // `/divert` put a ship and everyone inside it under way, and crew authority
    // — seats, proxies, musters, consent — is cut 10's (`force.assign`). The
    // READS are open to an actor who crews the carrier, because cut 5's whole
    // premise is that the focused station is the live control surface; a
    // bystander gets the byte-identical rule-4 `"No journey."`, so the read
    // leaks nothing.
    if ctx.switches.has("depart") {
        require_admin(ctx)?;
        journey_depart(ctx)
    } else if ctx.switches.has("divert") {
        require_admin(ctx)?;
        journey_divert(ctx)
    } else if ctx.switches.has("plan") {
        journey_plan(ctx)
    } else if ctx.switches.has("show") || !ctx.args.lhs().trim().is_empty() {
        journey_show(ctx)
    } else {
        journey_list(ctx)
    }
}

/// The dry run. 10b §3.6 drafts `/plan` as a durable step before `/depart`, but
/// RESOLUTION A's `JourneyState` has no `Planned` variant and inventing one
/// would put a second "is it moving?" answer beside `is_under_way`. So `/plan`
/// takes exactly `/depart`'s arguments, runs exactly its validation, prints the
/// legs with their IC boundaries, and **writes nothing** (recorded as a
/// deviation in §3.6 As built).
fn journey_plan(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Ok(carrier) = match_object(ctx.w, ctx.actor, ctx.args.lhs()) else {
        info(ctx, JOURNEY_NONE);
        return Ok(());
    };
    if !may_read_journey(ctx, carrier) {
        info(ctx, JOURNEY_NONE);
        return Ok(());
    }
    if let Some(why) = journey_ineligible(ctx, carrier) {
        return Err(CmdError::msg(why));
    }
    let spec = rhs_required(ctx, "@journey/plan <carrier>=<route id> <place>[:secs] …")?;
    let origin = origin_anchor(ctx, carrier)?;
    let (route, destination) = build_route(ctx, origin, &spec)?;
    refuse_unreachable_destination(ctx, carrier, &destination)?;
    let now = ctx.w.world_ic_now();
    let probe = CarrierJourney::new(
        JourneyId(0),
        carrier,
        route,
        destination,
        now.clone(),
        now,
        0,
        JourneyState::Cruising,
        None,
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;
    let lines = describe_journey(ctx, &probe, "Planned");
    for l in lines {
        info(ctx, l);
    }
    info(
        ctx,
        format!(
            "Nothing was written. @journey/depart #{}={spec} commits it.",
            carrier.0
        ),
    );
    Ok(())
}

fn journey_depart(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let carrier = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    if let Some(why) = journey_ineligible(ctx, carrier) {
        return Err(CmdError::msg(why));
    }
    let spec = rhs_required(ctx, "@journey/depart <carrier>=<route id> <place>[:secs] …")?;
    let origin = origin_anchor(ctx, carrier)?;
    let (route, destination) = build_route(ctx, origin, &spec)?;
    refuse_unreachable_destination(ctx, carrier, &destination)?;
    let now = ctx.w.world_ic_now();
    let id = ctx
        .w
        .module_state_mut::<JourneyBook>()
        .and_then(JourneyBook::alloc_id)
        .ok_or_else(|| CmdError::msg("The journey id allocator is exhausted."))?;
    let record = CarrierJourney::new(
        id,
        carrier,
        route,
        destination,
        now.clone(),
        now,
        0,
        JourneyState::Cruising,
        None,
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;
    let from = ctx.w.get(carrier).and_then(|o| o.location);
    let legs = record.route().legs().len();
    let due = record.arrival_at();
    // **The record goes into the book BEFORE the move** (E7 cut 6 fold, D2).
    // `graph::project` answers `in_transit_room` for a root the book says is
    // under way, and the projection is computed inside the move's plan — so
    // without this the ship, its exposed mounts and the " sets out." narrative
    // all stayed in the room it departed from for the whole passage. A refused
    // move rolls the entry back, so a refusal still mutates nothing.
    if let Some(book) = ctx.w.module_state_mut::<JourneyBook>() {
        book.journeys.insert(carrier, record.clone());
    }
    // The ONE checked carrier move: the root leaves its live-domain direct
    // placement and its whole subtree rides with it. A refusal mutates nothing,
    // and the allocator bump above is only a RAM cursor until the record below
    // persists it.
    let moved = match move_carrier_subtree(ctx, carrier, None) {
        Ok(moved) => moved,
        Err(error) => {
            if let Some(book) = ctx.w.module_state_mut::<JourneyBook>() {
                book.journeys.remove(&carrier);
            }
            return Err(sim_err(error));
        }
    };
    journey::persist_journey(ctx.out, &record);
    let (next_id, cursor) = {
        let book = jb(ctx);
        (book.next_id, book.cursor)
    };
    journey::persist_journey_meta(ctx.out, next_id, cursor);
    // Bump then refresh: the crew keeps the helm while the ship is under way.
    journey::refresh_carrier_links(ctx, carrier);
    if let Some(from) = from {
        // The departure half of the arrival's `Moved` (E7 cut 6 fold, R9(3)):
        // `move_carrier_subtree` mutates the world directly and only the `Ctx`
        // dispatch layer emits, so before the fold a ship's arrival had no
        // matching departure anywhere. Cut 6 adds no `WorldEvent` variant —
        // this is the one the tree already has.
        if let Some(to) = ctx.w.get(carrier).and_then(|o| o.location)
            && to != from
        {
            ctx.emit_world_event(&mush_core::WorldEvent::Moved {
                who: carrier,
                from: Some(from),
                to,
            });
        }
        let text = mush_proto::RichText::new()
            .span(mush_core::output::name_span(ctx.w, carrier))
            .text(" sets out.");
        ctx.out.to_room_from(
            from,
            vec![carrier],
            carrier,
            mush_proto::OutputEvent::Narrative {
                kind: mush_proto::NarrativeKind::Emit,
                actor: None,
                text,
            },
        );
    }
    info(
        ctx,
        format!(
            "{} departs on journey #{id}: {legs} leg(s), {} record(s) aboard, due at IC {}.",
            display_name(ctx.w, carrier),
            moved.len(),
            due.map_or("(overflow — the journey will page)".to_string(), |d| d
                .to_string()),
        ),
    );
    Ok(())
}

fn journey_divert(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let carrier = match_object(ctx.w, ctx.actor, ctx.args.lhs())?;
    // **A stopped journey diverts** (E7 cut 6 fold, R7). `journey::fail` writes
    // an anomaly whose recommended command is exactly this one, and before the
    // fold the `is_under_way` filter refused it — the page named the only
    // command the same code path would not run. An `Arrived` journey is done
    // and still refuses: that is a fresh `@journey/depart`.
    let current = jb(ctx)
        .get(carrier)
        .filter(|j| *j.state() != JourneyState::Arrived)
        .cloned()
        .ok_or_else(|| CmdError::msg("That carrier is not under way."))?;
    let spec = rhs_required(ctx, "@journey/divert <carrier>=<route id> <place>[:secs] …")?;
    // A carrier under way is not placed anywhere, so the re-plan keeps the
    // original origin endpoint: it has not landed, it has only changed its mind.
    let origin = current.route().endpoints().0;
    let (route, destination) = build_route(ctx, origin, &spec)?;
    refuse_unreachable_destination(ctx, carrier, &destination)?;
    let to = route.id().clone();
    let now = ctx.w.world_ic_now();
    let record = CarrierJourney::new(
        current.id(),
        carrier,
        route,
        destination,
        now.clone(),
        now,
        0,
        JourneyState::Diverted { to: to.clone() },
        None,
    )
    .map_err(|e| CmdError::msg(e.to_string()))?;
    journey::persist_journey(ctx.out, &record);
    let legs = record.route().legs().len();
    if let Some(book) = ctx.w.module_state_mut::<JourneyBook>() {
        book.journeys.insert(carrier, record);
    }
    // A carrier held after a stopped journey (D1) is under way again, so the
    // hold is released and it projects into the in-transit room rather than
    // sitting in the recovery room beside the wrecks.
    if st(ctx).is_quarantined(carrier)
        && let Err(error) = crate::transition::set_carrier_root_quarantine(
            ctx,
            carrier,
            false,
            "under way on a carrier journey",
        )
    {
        return Err(sim_err(error));
    }
    journey::refresh_carrier_links(ctx, carrier);
    info(
        ctx,
        format!(
            "{} is diverted onto `{to}`: {legs} leg(s) from here.",
            display_name(ctx.w, carrier)
        ),
    );
    Ok(())
}

/// Render a journey state for a viewer. Deliberately not `{:?}`: the derived
/// form prints `Captured { by: ObjId(n) }`, i.e. the captor's raw object id, to
/// anyone who can read the journey at all (E7 cut 6 fold, deviation (f)'s
/// residual). Cut 9 owns capture and will decide what a viewer may perceive
/// about a captor; until then the arm names the state and nothing else.
fn render_state(state: &JourneyState) -> String {
    match state {
        JourneyState::Cruising => "Cruising".to_string(),
        JourneyState::Promoted { domain } => format!("Promoted (domain #{domain})"),
        JourneyState::Diverted { to } => format!("Diverted (`{to}`)"),
        JourneyState::Arrived => "Arrived".to_string(),
        JourneyState::Captured { .. } => "Captured".to_string(),
        JourneyState::Failed => "Failed".to_string(),
    }
}

fn describe_journey(ctx: &Ctx<'_>, j: &CarrierJourney, lead: &str) -> Vec<String> {
    let mut lines = vec![format!(
        "{lead} journey #{} for {} on route `{}` — {}, leg {}/{}.",
        j.id(),
        display_name(ctx.w, j.root()),
        j.route().id(),
        render_state(j.state()),
        j.cursor(),
        j.route().legs().len(),
    )];
    for (k, leg) in j.route().legs().iter().enumerate() {
        let due = j
            .leg_due_at(k)
            .map_or("(overflow)".to_string(), |d| d.to_string());
        let mark = if u32::try_from(k).unwrap_or(u32::MAX) < j.cursor() {
            "*"
        } else {
            " "
        };
        lines.push(format!(
            "  {mark} leg {} → {} ({}s, IC {due})",
            k + 1,
            display_name(ctx.w, leg.to()),
            leg.base_s(),
        ));
    }
    lines.push(format!(
        "  destination: {}",
        describe_parent(ctx, j.destination())
    ));
    if let Some(anomaly) = j.anomaly() {
        lines.push(format!("  ANOMALY: {anomaly}"));
    }
    lines
}

/// A read. **Byte-identical** for a subject the viewer cannot see, a carrier
/// with no journey, and an object that never existed (rule 4): all three are
/// the one line below, so a probe learns nothing from the difference.
fn journey_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const NONE: &str = JOURNEY_NONE;
    let lhs = ctx.args.lhs().trim().to_string();
    let Ok(carrier) = match_object(ctx.w, ctx.actor, &lhs) else {
        info(ctx, NONE);
        return Ok(());
    };
    if !may_read_journey(ctx, carrier) {
        info(ctx, NONE);
        return Ok(());
    }
    let Some(journey) = jb(ctx).get(carrier).cloned() else {
        info(ctx, NONE);
        return Ok(());
    };
    for l in describe_journey(ctx, &journey, "Carrier") {
        info(ctx, l);
    }
    Ok(())
}

fn journey_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let staff = is_admin(ctx.w, ctx.actor);
    let roots: Vec<ObjId> = jb(ctx).journeys.keys().copied().collect();
    let visible: std::collections::BTreeSet<ObjId> = roots
        .into_iter()
        .filter(|root| staff || crews(ctx, ctx.actor, *root))
        .collect();
    let lines: Vec<String> = jb(ctx)
        .journeys
        .values()
        .filter(|j| visible.contains(&j.root()))
        .map(|j| {
            format!(
                "  #{} {} on `{}` — {}, leg {}/{}",
                j.id(),
                display_name(ctx.w, j.root()),
                j.route().id(),
                render_state(j.state()),
                j.cursor(),
                j.route().legs().len()
            )
        })
        .collect();
    if lines.is_empty() {
        info(ctx, "No carrier journeys.");
    } else {
        info(ctx, "Carrier journeys:");
        for l in lines {
            info(ctx, l);
        }
    }
    Ok(())
}

// ----------------------------------------------------------------------
// ---- E7c6: B5 — @domain/promote|demote|summary ----
// ----------------------------------------------------------------------

fn ab<'a>(ctx: &'a Ctx<'_>) -> &'a ActivationBook {
    ctx.w
        .module_state::<ActivationBook>()
        .expect("activation book slot installed at registration")
}

/// A lenient domain resolver for the READS, so a nonexistent domain and one the
/// viewer has never observed answer identically (rule 4).
fn resolve_domain_opt(ctx: &Ctx<'_>, s: &str) -> Option<DomainId> {
    let s = s.trim();
    if let Ok(n) = s.parse::<u64>() {
        return st(ctx)
            .domains
            .contains_key(&DomainId(n))
            .then_some(DomainId(n));
    }
    let host = match_object(ctx.w, ctx.actor, s).ok()?;
    st(ctx).domain_of_host(host).map(|d| d.id)
}

/// How many objects the domain currently holds — the one observable count cut 6
/// can put in a request or a summary.
fn domain_units(ctx: &Ctx<'_>, id: DomainId) -> u32 {
    u32::try_from(
        st(ctx)
            .derived
            .local_objects
            .get(&id)
            .map_or(0, BTreeSet::len),
    )
    .unwrap_or(u32::MAX)
}

/// Persist every request `drain_queue` promoted off the queue.
fn flush_dequeued(ctx: &mut Ctx<'_>) -> Vec<DomainId> {
    let promoted = match ctx.w.module_state_mut::<ActivationBook>() {
        Some(book) => domain::drain_queue(book),
        None => Vec::new(),
    };
    let records: Vec<DomainActivation> = promoted
        .iter()
        .filter_map(|id| ab(ctx).activations.get(id).cloned())
        .collect();
    for record in &records {
        domain::persist_activation(ctx.out, record);
    }
    promoted
}

fn domain_promote(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = resolve_domain(ctx, ctx.args.lhs())?;
    let status = st(ctx).domains.get(&id).map(|d| d.status);
    if status != Some(DomainStatus::Active) {
        return Err(CmdError::msg(format!(
            "Domain #{id} is not active; only an active domain promotes."
        )));
    }
    let token = ctx.args.rhs().map(str::trim).filter(|s| !s.is_empty());
    let trigger = match token {
        None => PromotionTrigger::PlayerPresent,
        Some(t) => PromotionTrigger::parse(t).ok_or_else(|| {
            CmdError::msg(
                "Unknown trigger. One of: player, contact, interception, operation.".to_string(),
            )
        })?,
    };
    // A fifth trigger on an already-promoted domain does NOT re-promote: the
    // request that promoted it stands, with its original trigger and stamp.
    if ab(ctx).is_promoted(id) {
        info(
            ctx,
            format!("Domain #{id} is already promoted; the standing request is unchanged."),
        );
        return Ok(());
    }
    let at = ctx.w.world_ic_now();
    let units = domain_units(ctx, id);
    // At the limit the trigger QUEUES and is re-evaluated — never dropped.
    let queued = ab(ctx).live() >= MAX_ACTIVE_DOMAINS;
    let record = DomainActivation::new(id, trigger, at, units, queued)
        .map_err(|e| CmdError::msg(e.to_string()))?;
    domain::persist_activation(ctx.out, &record);
    if let Some(book) = ctx.w.module_state_mut::<ActivationBook>() {
        book.activations.insert(id, record);
    }
    let promoted = flush_dequeued(ctx);
    if queued && !promoted.contains(&id) {
        info(
            ctx,
            format!(
                "Domain #{id} is QUEUED behind the {MAX_ACTIVE_DOMAINS} active-domain limit \
                 (trigger `{}`); it re-evaluates when a slot frees.",
                trigger.token()
            ),
        );
    } else {
        info(
            ctx,
            format!(
                "Domain #{id} requested promotion (trigger `{}`, {units} local). The \
                 activation is a REQUEST: cut 8's authority applies it and adds the pulse; \
                 nothing about the clock registry changed.",
                trigger.token()
            ),
        );
    }
    for other in promoted.into_iter().filter(|d| *d != id) {
        info(ctx, format!("Domain #{other} came off the queue."));
    }
    Ok(())
}

fn domain_demote(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = resolve_domain(ctx, ctx.args.lhs())?;
    let request = ab(ctx)
        .activations
        .get(&id)
        .filter(|a| !a.is_queued())
        .cloned()
        .ok_or_else(|| CmdError::msg(format!("Domain #{id} is not promoted.")))?;
    // The quiet boundary. Cut 6 owns no contacts, orders or damage — those
    // arrive with cuts 7-9 — but it DOES own the one pending thing that must
    // not be demoted out from under: a carrier still under way whose journey
    // ends in this domain.
    let inbound: Vec<ObjId> = jb(ctx)
        .journeys
        .values()
        .filter(|j| {
            j.state().is_under_way()
                && matches!(j.destination(), SpatialParent::Direct { domain, .. } if *domain == id)
        })
        .map(CarrierJourney::root)
        .collect();
    if let Some(first) = inbound.first() {
        return Err(CmdError::msg(format!(
            "Domain #{id} still has {} carrier(s) inbound (e.g. {}); it demotes only after a \
             quiet boundary ({QUIET_BOUNDARY}s with nothing pending).",
            inbound.len(),
            display_name(ctx.w, *first)
        )));
    }
    let to = ctx.w.world_ic_now();
    let units = domain_units(ctx, id);
    // **A world-IC rate change must not wedge a promoted domain** (E7 cut 6
    // fold, R4). The IC axis is CONTINUOUS across a generation
    // (`WorldTimeline::next_rate_epoch` sets `ic_anchor = current.ic_at(..)`),
    // so the window's two ends are directly comparable and only the generation
    // LABEL differs. Before the fold `DomainSummary::new`'s `ClockMismatch` arm
    // refused every demotion taken after any rate change — forever, across every
    // reboot, with the page recommending a generation that never comes back —
    // and the wedged domains accumulated against `MAX_ACTIVE_DOMAINS`.
    //
    // So the opening stamp is normalised onto the closing generation at this
    // ONE construction site: the elapsed IC window stays exact to the second,
    // and the record documents a cross-generation window rather than refusing
    // to exist. (`min` keeps `WindowBackwards` unreachable; a backwards window
    // would mean the IC axis itself ran backwards.)
    let opened_generation = match request.at().clock {
        ClockRef::WorldIc { generation } => Some(generation),
        _ => None,
    };
    let from = ClockStamp {
        clock: to.clock.clone(),
        at: request.at().at.min(to.at),
    };
    if from.clock != request.at().clock {
        tracing::info!(
            domain = %id,
            opened_generation = ?opened_generation,
            "sim: a domain summary window spans a world-IC rate generation; the opening stamp \
             is normalised onto the closing generation (the IC axis is continuous)"
        );
    }
    // The summary is written BEFORE the cadence changes, and a summary that
    // will not construct leaves the domain PROMOTED and pages — never
    // demoted-without-summary.
    let summary = match DomainSummary::new(id, from, to, 0, units) {
        Ok(summary) => summary,
        Err(error) => {
            tracing::error!(
                domain = %id,
                %error,
                "sim: a domain summary would not construct; the domain STAYS PROMOTED. \
                 Recommended: @domain/status to read its window, then @domain/demote again"
            );
            return Err(CmdError::msg(format!(
                "Domain #{id} could not write its summary ({error}); it stays promoted and staff \
                 have been paged."
            )));
        }
    };
    if !domain::persist_summary(ctx.out, &summary) {
        return Err(CmdError::msg(format!(
            "Domain #{id} could not write its summary; it stays promoted and staff have been \
             paged."
        )));
    }
    domain::persist_activation_del(ctx.out, id);
    let elapsed = summary.elapsed();
    if let Some(book) = ctx.w.module_state_mut::<ActivationBook>() {
        book.summaries.insert(id, summary);
        book.activations.remove(&id);
    }
    let promoted = flush_dequeued(ctx);
    info(
        ctx,
        format!(
            "Domain #{id} demoted; its summary is written first ({elapsed}s of IC window, \
             {units} local). Demotion changes cadence, never state."
        ),
    );
    for other in promoted {
        info(ctx, format!("Domain #{other} came off the queue."));
    }
    Ok(())
}

/// A read. **Byte-identical** for a domain nobody has observed and one that
/// never existed (rule 4).
fn domain_summary(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const NONE: &str = "No summary for that domain.";
    let lines = match resolve_domain_opt(ctx, ctx.args.lhs())
        .and_then(|id| ab(ctx).summaries.get(&id).cloned())
    {
        None => vec![NONE.to_string()],
        Some(s) => vec![
            format!(
                "Domain #{} — tactical window IC {} → {} ({}s).",
                s.domain(),
                s.from().at,
                s.to().at,
                s.elapsed()
            ),
            format!(
                "  {} contact(s), {} local at close.",
                s.contacts(),
                s.units()
            ),
        ],
    };
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

// ----------------------------------------------------------------------
// ---- E7c8: B6 — @domain/tick|replay (switches on cut 6's baseline @domain) ----
// ----------------------------------------------------------------------

/// `@domain/tick <id|host>` (staff) — single-step the applying authority.
///
/// It runs exactly the maintenance pass the `sim.domain_pulse` Coarse tick runs
/// — applying a live `DomainActivation` request, arming the durable cursor, or
/// standing a no-longer-eligible domain down — for **one** domain, and reports
/// what it decided. It writes one `sim` domain document and, at most once,
/// submits this module's own `domain_pulse_arm` action; it **never touches the
/// composition-frozen `ClockProviderRegistry`**.
///
/// `@domain/tick` on a `Passive` domain with no live activation request
/// **refuses**: promotion is `@domain/promote`'s, and this is its apply half.
fn domain_tick(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = resolve_domain(ctx, ctx.args.lhs())?;
    let pulsing = st(ctx)
        .domains
        .get(&id)
        .is_some_and(|record| record.clock.is_pulsing());
    if !pulsing && !ab(ctx).is_promoted(id) {
        return Err(CmdError::msg(format!(
            "Domain #{id} is passive and has no live activation request; @domain/promote it first."
        )));
    }
    let plan = crate::tick::run_maintenance(ctx, Some(id));
    for refusal in &plan.refused {
        info(ctx, refusal.clone());
    }
    if plan.applied.contains(&id) {
        info(
            ctx,
            format!(
                "Domain #{id} now pulses; its durable tick cursor is being armed. The clock \
                 provider registry is unchanged — it was frozen at composition."
            ),
        );
    } else if plan.armed.contains(&id) {
        info(
            ctx,
            format!(
                "Domain #{id} already pulses; its tick cursor is armed (a duplicate arm is a HIT)."
            ),
        );
    } else if plan.stood_down.contains(&id) {
        info(
            ctx,
            format!(
                "Domain #{id} is no longer eligible and was stood down to Passive; its state is untouched."
            ),
        );
    } else if plan.refused.is_empty() {
        info(ctx, format!("Domain #{id}: nothing to do this pass."));
    }
    Ok(())
}

/// `@domain/replay <id|host>=<from>-<to>` (staff) — read back the retained
/// deterministic commits.
///
/// A **read**: it prints each retained tick's state hash, accepted count and
/// recorded draw count out of the `domain_tick_commits` family, which is how a
/// person checks that a replay reproduced the stored hash. It resolves nothing
/// and writes nothing.
fn domain_replay(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    const NONE: &str = "No retained ticks in that range.";
    let Some(id) = resolve_domain_opt(ctx, ctx.args.lhs()) else {
        info(ctx, NONE);
        return Ok(());
    };
    let (from, to) = parse_tick_range(ctx.args.rhs().unwrap_or_default()).ok_or_else(|| {
        CmdError::msg("Usage: @domain/replay <id|host>=<from>-<to> (tick numbers).")
    })?;
    let Some(handles) = ctx.w.module_state::<crate::tick::TickHandles>().cloned() else {
        info(ctx, NONE);
        return Ok(());
    };
    // The commit key carries the clock generation since the E7c8 fold (D5(a)),
    // so the replay reads the domain's CURRENT generation — a re-promoted
    // domain's tick 5 is a different record from the previous generation's.
    let generation = st(ctx)
        .domains
        .get(&id)
        .and_then(|record| record.clock.generation())
        .unwrap_or(0);
    let mut lines = Vec::new();
    let mut refusals = 0_usize;
    // Bounded on records **visited** (D-LBK-5): the retention ring is
    // `MAX_RETAINED_TICK_COMMITS`, so the walk can never exceed it.
    for step in 0..crate::tick::MAX_RETAINED_TICK_COMMITS {
        let Some(want) = from.checked_add(step as i64) else {
            break;
        };
        if want > to {
            break;
        }
        let key = crate::tick::TickCommitKey {
            domain: id,
            generation,
            tick: want,
        };
        if let Ok(Some(commit)) = ctx.action_state(&handles.commits, &key) {
            let refused = commit
                .deltas()
                .iter()
                .filter(|delta| matches!(delta.outcome, crate::tick::DeltaOutcome::Refused { .. }))
                .count();
            refusals = refusals.saturating_add(refused);
            lines.push(format!(
                "  tick {:>6}  state {}  {} accepted, {} delta(s) ({} refused), {} draw(s), {} \
                 fact(s)",
                commit.tick(),
                commit.state_hash().to_hex(),
                commit.accepted().len(),
                commit.deltas().len(),
                refused,
                commit.draws().len(),
                commit.facts().len()
            ));
        }
    }
    if refusals > 0 {
        // **T12/A4:** a refusal is surfaced to the operator, not buried in the
        // record. A boundary-fact budget exhaustion reads here as a refusal
        // with its count rather than as a silent truncation.
        lines.push(format!(
            "  {refusals} intent(s) in that range were REFUSED with a recorded reason; they were \
             never accepted."
        ));
    }
    if lines.is_empty() {
        info(ctx, NONE);
        return Ok(());
    }
    info(ctx, format!("Domain #{id} — retained ticks {from}..={to}:"));
    for line in lines {
        info(ctx, line);
    }
    Ok(())
}

fn parse_tick_range(raw: &str) -> Option<(i64, i64)> {
    let raw = raw.trim();
    let (from, to) = raw.split_once('-')?;
    let from: i64 = from.trim().parse().ok()?;
    let to: i64 = to.trim().parse().ok()?;
    (from <= to).then_some((from, to))
}

/// Register `@domain`, `@spatial`, `@journey` and cut 5's focus surface.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    // **E7 cut 8 independent verify fold B1.** `@domain` is one `CommandSpec`,
    // so it takes the CEILING class of its switches — and `/tick` reaches an
    // `ActionTxn`: `cmd_domain` → `domain_tick` → `tick::run_maintenance` →
    // `ctx.submit_authoritative(domain_pulse_arm, ..)`. Cut 8's own acceptance
    // test proved it by calling `ok("@domain/tick")` and then draining the
    // submission the command had just queued, while `modules.rs` pinned it
    // `LegacyPostImage` with the note "the durable cursor is armed by the
    // `sim.domain_pulse` Coarse tick, **never by the command**". That is the
    // identical misdeclaration the tripwire caught for `@station` at cut 5 and
    // `@prop` at cut 7. `LegacyPostImage` is restored immediately after, so
    // every later registration keeps the module default.
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: "@domain".into(),
            shape: ArgShape::EqPair,
            switches: [
                "create", "status", "pause", "resume", "retire", "present", "promote", "demote",
                "summary", "tick", "replay",
            ]
            .iter()
            .copied()
            .map(SmolStr::new)
            .collect(),
            help: "@domain  --  (staff) Simulation domains. No switch: list.\n\
                   @domain/create <host>=<driver>[,<presentation room>]   (driver: sim.passive)\n\
                   @domain/status <id|host>   @domain/pause|/resume <id|host>\n\
                   @domain/retire <id|host>   (must be unreferenced; then @behavior/remove sim.domain)\n\
                   @domain/present <id|host>=<room>   (re-point the presentation room)\n\
                   @domain/promote <id|host>[=<trigger>]   Request tactical cadence (trigger: \
                   player|contact|interception|operation). Writes a REQUEST; the pulse itself is \
                   a later cut's, and the clock registry is never touched.\n\
                   @domain/demote <id|host>   Write the durable summary, then stand down.\n\
                   @domain/summary <id|host>   Read the last tactical window.\n\
                   @domain/tick <id|host>   Single-step the applying authority: apply a live \
                   activation request, arm the durable tick cursor, or stand a no-longer-eligible \
                   domain down. Never touches the frozen clock provider registry.\n\
                   @domain/replay <id|host>=<from>-<to>   Read back the retained deterministic \
                   tick commits and their state hashes."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_domain),
    )?;
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    reg.command(
        CommandSpec {
            name: "@spatial".into(),
            shape: ArgShape::EqPair,
            switches: [
                "place", "domain", "interior", "attached", "unplace", "keep", "space", "unspace", "slot",
                "mount", "unmount", "station", "unstation", "assign", "unassign", "authority", "egress",
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
            help:
                "egress  --  Leave the interior space you are placed inside, onto the surrounding \
                   simulation domain (a mech egressing a dropship bay, a trooper leaving an APC)."
                    .into(),
            ..CommandSpec::default()
        },
        Arc::new(egress_cmd),
    )?;
    reg.command(
        CommandSpec {
            name: "@route".into(),
            shape: ArgShape::EqPair,
            switches: ["show", "pause", "resume", "abort"].iter().copied().map(SmolStr::new).collect(),
            help: "@route  --  (staff) Send a plain object along a legible, interceptable route of rooms.\n                   @route <traveler>=<room>[:secs] <room>[:secs] ...   (default 30s/leg)\n                   @route/show <traveler>   @route/pause|/resume|/abort <traveler>   @route (no arg: list)"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_route),
    )?;
    reg.command(
        CommandSpec {
            name: "+intercept".into(),
            aliases: vec!["waylay".into()],
            shape: ArgShape::Raw,
            help: "+intercept <traveler>  --  Stop a traveler passing through your room (you must be here to see it)."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(intercept_cmd),
    )?;

    // E7c6 (B6): `@journey` is one `CommandSpec`, so it is ONE registration and
    // takes the CEILING class of its switches. `/show` is read-only and `/plan`
    // is a dry run that writes nothing, but `/depart` and `/divert` write the
    // exempt `sim.journey` record (`cjourney:<root>`) — a bounded whole-document
    // post-image with a strict `TryFrom` wire and a deterministic key — so the
    // command publishes `LegacyPostImage`. It takes no part in authority or
    // prepared mechanics: the placement it moves is the pre-existing synchronous
    // `SpatialState` lane through the one checked `move_carrier_subtree`, and
    // the typed placement family is E7k's (08h Q5). K8.5 requires the class
    // before the registration; `LegacyPostImage` is restored afterwards so any
    // later mush-sim registration keeps the module default.
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    reg.command(
        CommandSpec {
            name: "@journey".into(),
            shape: ArgShape::EqPair,
            switches: ["plan", "depart", "show", "divert"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@journey  --  (staff) Carrier journeys: a ship and everything aboard it move \
                   together along a route. No switch, no argument: list.\n\
                   @journey/plan <carrier>=<route id> <place>[:secs] …    Dry run — validates \
                   and prints the legs, writes nothing.\n\
                   @journey/depart <carrier>=<route id> <place>[:secs] …  Set out. The carrier \
                   leaves its domain placement; its interiors, occupants, mounts and crew ride \
                   with it and its crew keep the helm.\n\
                   @journey/divert <carrier>=<route id> <place>[:secs] …  Re-plan a carrier that \
                   is under way.\n\
                   @journey/show <carrier>                                Show one journey."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_journey),
    )?;

    // E7c5 (B8): focus / billets / identity. K8.5 requires a classification
    // before each `reg.command`; every command stores the class captured at its
    // own registration. `+stations` and `@identity/audit` are ReadOnly; `+focus`
    // is LegacyPostImage (it persists only `cgen`). `LegacyPostImage` is restored
    // afterwards so any later mush-sim registration keeps the module default.
    reg.classify_commands(CommandCompatibility::ReadOnly);
    reg.command(
        CommandSpec {
            name: "+stations".into(),
            shape: ArgShape::Raw,
            help: "+stations  --  Your control billets, with reach and which one you drive.".into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_stations),
    )?;
    reg.command(
        CommandSpec {
            name: "@identity".into(),
            shape: ArgShape::EqPair,
            switches: ["audit"].iter().copied().map(SmolStr::new).collect(),
            help: "@identity/audit <a>=<b>  --  (staff) Do two characters share one account? \
                   Answers same/different and never an identity."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_identity),
    )?;
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    reg.command(
        CommandSpec {
            name: "+focus".into(),
            shape: ArgShape::Raw,
            switches: ["release"].iter().copied().map(SmolStr::new).collect(),
            help: "+focus <unit>/<station>  --  Take foreground control of one of your billets \
                   (atomically releasing your previous focus).\n\
                   +focus/release  --  Give up foreground control.   +focus (no arg): show it."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_focus),
    )?;
    // E7c5 (B5/B6/B8): `@station` is one CommandSpec, so it takes the CEILING
    // class of its switches — `/handoff` and `/cancel` ride cut 2's authoritative
    // ingress, so the command publishes `Authoritative` (as `@inv`/`@wo` do).
    // `/show` is read-only and `/fallback` writes the exempt `sim.station_fallback`
    // record; both live under the one authoritative lane. `LegacyPostImage` is
    // restored afterwards so any later mush-sim registration keeps the default.
    reg.classify_commands(CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: "@station".into(),
            shape: ArgShape::EqPair,
            switches: ["handoff", "cancel", "show", "fallback"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@station  --  Station control leases (K6).\n\
                   @station/handoff <unit>/<station>   Take the control lease for the station \
                   you are driving (the first request establishes a vacant lease).\n\
                   @station/cancel <unit>/<station>    Withdraw your still-pending handoff.\n\
                   @station/show [<unit>/<station>]    Show the station you are driving.\n\
                   @station/fallback <unit>/<station>=<who>   (staff) Declare its fallback \
                   controller."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(crate::station::cmd_station),
    )?;
    // E7c8 B6: the eight tactical verbs join the SAME authoritative group —
    // every one of them stages `tactical_intent` through cut 2's seam, which is
    // the ceiling class `@station` and `@prop` already set. K8.5's rule is
    // "classify before the first `reg.command` of the group", and this is inside
    // the group `@station` opened.
    crate::combat::register(reg)?;
    reg.classify_commands(CommandCompatibility::LegacyPostImage);
    Ok(())
}

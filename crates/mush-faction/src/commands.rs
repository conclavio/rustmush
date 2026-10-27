//! The faction command surface.
//!
//! Staff (`Royalty+` or the `FACTION_ADMIN` power): `@faction/*` and
//! `@region/*` administer the registry — truth lives here. Players:
//! `+factions`, `+faction <name>`, `+territory` — every player-facing render
//! projects the registry through membership/edge/control [`Visibility`]
//! policies. The registry never leaks: a hidden faction is indistinguishable
//! from a missing one, a covert membership from no membership.

use std::sync::Arc;

use mush_core::output::display_name;
use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, Flags, ModError, ObjId, ObjKind, Powers, Registrar, Role,
    World, perm,
};
use smol_str::SmolStr;

use crate::data::{
    ControlKind, ControlRecord, EdgeKind, Faction, FactionEdge, FactionId, FactionMembership,
    FactionState, MAX_NAME_LEN, Region, RegionId, Visibility, persist_control, persist_del,
    persist_faction, persist_membership, persist_meta, persist_region,
};
use crate::data::{control_key, fac_key, member_key, region_key};

// ----------------------------------------------------------------------
// Shared helpers
// ----------------------------------------------------------------------

/// The mutable faction state slot (installed at engine construction).
fn state<'a>(ctx: &'a mut Ctx<'_>) -> &'a mut FactionState {
    ctx.w
        .module_state_mut::<FactionState>()
        .expect("faction state slot installed at registration")
}

/// The shared faction state slot.
fn state_ref<'a>(ctx: &'a Ctx<'_>) -> &'a FactionState {
    ctx.w
        .module_state::<FactionState>()
        .expect("faction state slot installed at registration")
}

/// Whether `who` may administer the registry: the `FACTION_ADMIN` power
/// (Wizard+ implicitly) or Royalty+.
fn is_admin(w: &World, who: ObjId) -> bool {
    perm::has_power(w, who, Powers::FACTION_ADMIN) || perm::has_role(w, who, Role::Royalty)
}

fn require_admin(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if is_admin(ctx.w, ctx.actor) {
        Ok(())
    } else {
        Err(CmdError::msg("Permission denied."))
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '\'' | '.'))
}

fn parse_bool(s: &str) -> Result<bool, CmdError> {
    match s.trim().to_ascii_lowercase().as_str() {
        "yes" | "true" | "1" | "on" => Ok(true),
        "no" | "false" | "0" | "off" => Ok(false),
        other => Err(CmdError::msg(format!("Not a yes/no value: {other:?}."))),
    }
}

/// Resolve a faction by name/alias, honoring the hidden gate for `viewer`
/// (a hidden faction is indistinguishable from a missing one).
pub(crate) fn resolve_faction(
    ctx: &Ctx<'_>,
    name: &str,
    honor_hidden: bool,
) -> Result<Faction, CmdError> {
    let staff = is_admin(ctx.w, ctx.actor);
    let st = state_ref(ctx);
    let f = st
        .faction_by_name(name)
        .ok_or_else(|| CmdError::msg("No such faction."))?;
    if honor_hidden && !st.faction_listed(ctx.actor, staff, f) {
        return Err(CmdError::msg("No such faction."));
    }
    Ok(f.clone())
}

fn resolve_region(ctx: &Ctx<'_>, name: &str) -> Result<Region, CmdError> {
    state_ref(ctx)
        .region_by_name(name)
        .cloned()
        .ok_or_else(|| CmdError::msg("No such region."))
}

pub(crate) fn rhs_required(ctx: &Ctx<'_>, usage: &str) -> Result<String, CmdError> {
    ctx.args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .ok_or_else(|| CmdError::msg(format!("Usage: {usage}")))
}

/// Split a `key:value` (or `key=value`) modifier segment.
fn split_kv(seg: &str) -> Option<(&str, &str)> {
    seg.split_once(':')
        .or_else(|| seg.split_once('='))
        .map(|(k, v)| (k.trim(), v.trim()))
}

fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "faction", msg.into());
}

/// Whether a membership visibility auto-joins the faction channel. A comsys
/// roster is a world-readable display surface (`@channel/who` and `clist`
/// obey only the channel's see lock, which defaults open), so **only Public
/// members ride it** — auto-joining even a MembersKnow member would publish
/// exactly the membership the registry hides from non-members. Members with
/// quieter visibilities may still `addcom` by hand; that is self-disclosure,
/// and the module never touches a membership it did not create.
fn channel_eligible(vis: Visibility) -> bool {
    matches!(vis, Visibility::Public)
}

/// The faction's bound channel and display name, if any.
fn bound_channel(ctx: &Ctx<'_>, faction: FactionId) -> Option<(mush_comsys::ChannelId, String)> {
    let st = state_ref(ctx);
    st.factions
        .get(&faction)
        .and_then(|f| f.channel.map(|c| (c, f.name.to_string())))
}

/// Join `player` to the bound channel iff their membership is
/// channel-eligible. Never leaves — a manual join by a quieter-visibility
/// member is not the module's to remove.
fn channel_join_if_eligible(ctx: &mut Ctx<'_>, faction: FactionId, player: ObjId) {
    let Some((chan, fname)) = bound_channel(ctx, faction) else {
        return;
    };
    let eligible = state_ref(ctx)
        .membership(player, faction)
        .is_some_and(|m| channel_eligible(m.visibility));
    if eligible {
        mush_comsys::api::force_join(ctx, player, chan, &fname);
    }
}

// ----------------------------------------------------------------------
// @faction — staff administration
// ----------------------------------------------------------------------

fn cmd_faction(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_admin(ctx)?;
    if ctx.switches.has("create") {
        faction_create(ctx)
    } else if ctx.switches.has("destroy") {
        faction_destroy(ctx)
    } else if ctx.switches.has("desc") {
        faction_desc(ctx)
    } else if ctx.switches.has("set") {
        faction_set(ctx)
    } else if ctx.switches.has("add") {
        faction_add(ctx)
    } else if ctx.switches.has("mod") {
        faction_mod(ctx)
    } else if ctx.switches.has("remove") {
        faction_remove(ctx)
    } else if ctx.switches.has("edge") {
        faction_edge(ctx)
    } else if ctx.switches.has("unedge") {
        faction_unedge(ctx)
    } else if ctx.switches.has("principal") {
        faction_principal(ctx)
    } else if ctx.switches.has("resource") {
        faction_resource(ctx)
    } else if ctx.switches.has("org") {
        crate::econ::faction_org(ctx)
    } else if ctx.switches.has("migrate") {
        crate::econ::faction_migrate(ctx)
    } else if ctx.switches.has("channel") {
        faction_channel(ctx)
    } else if ctx.switches.has("control") {
        faction_control(ctx)
    } else if ctx.switches.has("uncontrol") {
        faction_uncontrol(ctx)
    } else if ctx.switches.has("contest") {
        faction_contest(ctx)
    } else if ctx.switches.has("incident") {
        crate::reputation::command_incident(ctx)
    } else if ctx.switches.has("publish-incident") {
        crate::reputation::command_publish(ctx)
    } else if ctx.switches.has("show") {
        faction_show(ctx)
    } else {
        faction_staff_list(ctx)
    }
}

fn faction_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let name = ctx.args.lhs().trim().to_string();
    if !valid_name(&name) {
        return Err(CmdError::msg(format!(
            "That is not a usable faction name (1-{MAX_NAME_LEN} letters, digits, space, - _ ' .)."
        )));
    }
    if state_ref(ctx).faction_by_name(&name).is_some() {
        return Err(CmdError::msg("A faction by that name already exists."));
    }
    let (f, next_f, next_r) = {
        let st = state(ctx);
        let id = st.alloc_faction_id();
        let f = Faction::new(id, &name);
        st.factions.insert(id, f.clone());
        (f, st.next_faction_id, st.next_region_id)
    };
    persist_faction(ctx.out, &f);
    persist_meta(ctx.out, next_f, next_r);
    info(ctx, format!("Created faction {name} (id {}).", f.id));
    Ok(())
}

fn faction_destroy(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let f = resolve_faction(ctx, ctx.args.lhs(), false)?;
    // E3d / D-E3-8: a bound faction is an attribution for money that outlives it.
    // The cascade below deletes the faction doc, which would silently orphan the
    // link — and with it the only record of WHOSE the org's balances were. Unbinding
    // first is a deliberate staff act that leaves the organization and its accounts
    // standing; only then may the registry entry go. (`f.org` is read straight from
    // the doc, not through `backing_org`: a MISSING or CONFLICTing link must block
    // destruction too, precisely because it is the state that needs attention.)
    if let Some(org) = f.org {
        return Err(CmdError::msg(format!(
            "{} is backed by org #{org}; unbind it first (@faction/org {}=none). \
             The organization and its accounts survive the unbind.",
            f.name, f.name
        )));
    }
    let id = f.id;
    // Capture the bound channel and the module-joined (eligible) members
    // BEFORE the cascade deletes the faction — they leave the channel below,
    // mirroring the single-member @faction/remove path.
    let channel_leavers: Vec<(mush_comsys::ChannelId, ObjId)> = bound_channel(ctx, id)
        .map(|(chan, _)| {
            state_ref(ctx)
                .members_of(id)
                .filter(|m| channel_eligible(m.visibility))
                .map(|m| (chan, m.character))
                .collect()
        })
        .unwrap_or_default();
    // Collect every cascading change first, then apply + persist.
    let (member_keys, edge_owners, control_regions) = {
        let st = state(ctx);
        let member_keys: Vec<(ObjId, FactionId)> = st
            .members_of(id)
            .map(|m| (m.character, m.faction))
            .collect();
        for k in &member_keys {
            st.members.remove(k);
        }
        let mut edge_owners: Vec<Faction> = Vec::new();
        for other in st.factions.values_mut() {
            if other.id != id && other.edges.iter().any(|e| e.to == id) {
                other.edges.retain(|e| e.to != id);
                edge_owners.push(other.clone());
            }
        }
        let mut control_regions: Vec<(RegionId, Vec<ControlRecord>)> = Vec::new();
        for (region, recs) in &mut st.control {
            let before = recs.len();
            recs.retain(|r| r.faction != id);
            let mut changed = recs.len() != before;
            for r in recs.iter_mut() {
                let b = r.contested_by.len();
                r.contested_by.retain(|c| *c != id);
                changed |= r.contested_by.len() != b;
            }
            if changed {
                control_regions.push((*region, recs.clone()));
            }
        }
        st.control.retain(|_, recs| !recs.is_empty());
        st.factions.remove(&id);
        (member_keys, edge_owners, control_regions)
    };
    for (p, fac) in &member_keys {
        persist_del(ctx.out, member_key(*p, *fac));
    }
    for owner in &edge_owners {
        persist_faction(ctx.out, owner);
    }
    for (region, recs) in &control_regions {
        persist_control(ctx.out, *region, recs);
    }
    persist_del(ctx.out, fac_key(id));
    // Prune the faction's simulation state (fronts + clocks) so it leaves no
    // dangling sim docs — the second state slot the FactionState cascade above
    // doesn't touch.
    let sim_dels = ctx
        .w
        .module_state_mut::<crate::SimState>()
        .map(|s| crate::sim::prune_faction(s, id))
        .unwrap_or_default();
    for key in sim_dels {
        crate::sim::persist_del(ctx.out, key);
    }
    for (chan, who) in channel_leavers {
        mush_comsys::api::force_leave(ctx, who, chan);
    }
    info(
        ctx,
        format!(
            "Destroyed faction {} ({} membership{} removed).",
            f.name,
            member_keys.len(),
            if member_keys.len() == 1 { "" } else { "s" }
        ),
    );
    Ok(())
}

fn faction_desc(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let f = resolve_faction(ctx, ctx.args.lhs(), false)?;
    let charter = ctx.args.rhs().unwrap_or("").trim().to_string();
    let updated = {
        let st = state(ctx);
        let fac = st.factions.get_mut(&f.id).expect("resolved above");
        fac.charter = charter;
        fac.clone()
    };
    persist_faction(ctx.out, &updated);
    info(ctx, format!("Charter set on {}.", updated.name));
    Ok(())
}

fn faction_set(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let f = resolve_faction(ctx, ctx.args.lhs(), false)?;
    let rhs = rhs_required(ctx, "@faction/set <faction>=<field>:<value>")?;
    let (field, value) = split_kv(&rhs)
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .ok_or_else(|| CmdError::msg("Usage: @faction/set <faction>=<field>:<value>"))?;
    let field = field.to_ascii_lowercase();
    let value = value.as_str();
    // Aliases join the same case-insensitive namespace as faction names and
    // resolve through the same lookup, so they get the same validation as
    // @faction/create: legal charset, and no collision with any OTHER
    // faction's name or alias — an unchecked alias would silently re-target
    // every name-resolving command (including @faction/destroy).
    let parsed_aliases: Option<Vec<SmolStr>> = if field == "aliases" {
        let parsed: Vec<SmolStr> = value
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(SmolStr::new)
            .collect();
        let st = state_ref(ctx);
        for a in &parsed {
            if !valid_name(a) {
                return Err(CmdError::msg(format!(
                    "Alias {a:?} is not a usable name \
                     (1-{MAX_NAME_LEN} letters, digits, space, - _ ' .)."
                )));
            }
            if let Some(other) = st.faction_by_name(a)
                && other.id != f.id
            {
                return Err(CmdError::msg(format!(
                    "Alias {a:?} collides with faction {}.",
                    other.name
                )));
            }
        }
        Some(parsed)
    } else {
        None
    };
    let updated = {
        let st = state(ctx);
        let fac = st.factions.get_mut(&f.id).expect("resolved above");
        match field.as_str() {
            "hidden" => fac.hidden = parse_bool(value)?,
            "officer" | "officer_level" => {
                fac.officer_level = value
                    .parse::<u8>()
                    .map_err(|_| CmdError::msg("officer level must be 0-255."))?;
            }
            "intel" | "intel_level" => {
                fac.intel_level = value
                    .parse::<u8>()
                    .map_err(|_| CmdError::msg("intel level must be 0-255."))?;
            }
            "aliases" => {
                fac.aliases = parsed_aliases.clone().expect("validated above");
            }
            "ext" => {
                fac.ruleset_ext = if value.is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::from_str(value)
                        .map_err(|e| CmdError::msg(format!("ext must be JSON: {e}")))?
                };
            }
            _ => {
                return Err(CmdError::msg(
                    "Unknown field. Fields: hidden, officer, intel, aliases, ext.",
                ));
            }
        }
        fac.clone()
    };
    persist_faction(ctx.out, &updated);
    info(ctx, format!("Set {field} on {}.", updated.name));
    Ok(())
}

/// A parsed membership spec: the resolved player plus the level /
/// visibility / title overrides present.
type MemberSpec = (ObjId, Option<u8>, Option<Visibility>, Option<SmolStr>);

/// Parse `player[/key:val...]` membership modifiers.
fn parse_member_spec(ctx: &Ctx<'_>, spec: &str) -> Result<MemberSpec, CmdError> {
    let mut segs = spec.split('/').map(str::trim);
    let who_name = segs
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Name a player."))?;
    let who = mush_core::match_object(ctx.w, ctx.actor, who_name)?;
    let (mut level, mut vis, mut title) = (None, None, None);
    for seg in segs {
        let Some((k, v)) = split_kv(seg) else {
            return Err(CmdError::msg(format!(
                "Bad modifier {seg:?} (want level:<n>, vis:<v>, or title:<t>)."
            )));
        };
        match k.to_ascii_lowercase().as_str() {
            "level" => {
                level = Some(
                    v.parse::<u8>()
                        .map_err(|_| CmdError::msg("level must be 0-255."))?,
                );
            }
            "vis" | "visibility" => {
                vis = Some(Visibility::parse(v).ok_or_else(|| {
                    CmdError::msg("visibility: public, members, officers, or covert.")
                })?);
            }
            "title" => title = Some(SmolStr::new(v)),
            other => {
                return Err(CmdError::msg(format!("Unknown modifier {other:?}.")));
            }
        }
    }
    Ok((who, level, vis, title))
}

fn faction_add(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let f = resolve_faction(ctx, ctx.args.lhs(), false)?;
    let rhs = rhs_required(
        ctx,
        "@faction/add <faction>=<player>[/level:<n>][/vis:<v>][/title:<t>]",
    )?;
    let (who, level, vis, title) = parse_member_spec(ctx, &rhs)?;
    let kind = ctx.w.get(who).map(|o| o.kind);
    if !matches!(kind, Some(ObjKind::Player | ObjKind::Npc)) {
        return Err(CmdError::msg("Only players and NPCs join factions."));
    }
    if state_ref(ctx).is_member(who, f.id) {
        return Err(CmdError::msg(
            "Already a member — use @faction/mod to change level/visibility/title.",
        ));
    }
    let now = ctx.w.now();
    let m = FactionMembership {
        character: who,
        faction: f.id,
        title: title.unwrap_or_default(),
        level: level.unwrap_or(1),
        visibility: vis.unwrap_or_default(),
        since: now,
    };
    state(ctx).members.insert((who, f.id), m.clone());
    persist_membership(ctx.out, &m);
    channel_join_if_eligible(ctx, f.id, who);
    let name = display_name(ctx.w, who);
    info(
        ctx,
        format!(
            "Added {name} to {} (level {}, {}).",
            f.name,
            m.level,
            m.visibility.label()
        ),
    );
    Ok(())
}

fn faction_mod(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let f = resolve_faction(ctx, ctx.args.lhs(), false)?;
    let rhs = rhs_required(
        ctx,
        "@faction/mod <faction>=<player>[/level:<n>][/vis:<v>][/title:<t>]",
    )?;
    let (who, level, vis, title) = parse_member_spec(ctx, &rhs)?;
    let (m, was_eligible) = {
        let st = state(ctx);
        let Some(m) = st.members.get_mut(&(who, f.id)) else {
            return Err(CmdError::msg("Not a member of that faction."));
        };
        let was_eligible = channel_eligible(m.visibility);
        if let Some(l) = level {
            m.level = l;
        }
        if let Some(v) = vis {
            m.visibility = v;
        }
        if let Some(t) = title {
            m.title = t;
        }
        (m.clone(), was_eligible)
    };
    persist_membership(ctx.out, &m);
    // Touch the bound channel only when the visibility actually changed
    // eligibility — a title- or level-only edit must not kick a member's
    // manual channel join, nor silently revert a deliberate delcom.
    if vis.is_some() {
        let now_eligible = channel_eligible(m.visibility);
        if !was_eligible && now_eligible {
            channel_join_if_eligible(ctx, f.id, who);
        } else if was_eligible
            && !now_eligible
            && let Some((chan, _)) = bound_channel(ctx, f.id)
        {
            mush_comsys::api::force_leave(ctx, who, chan);
        }
    }
    let name = display_name(ctx.w, who);
    info(
        ctx,
        format!(
            "{name} in {}: level {}, {}, title {:?}.",
            f.name,
            m.level,
            m.visibility.label(),
            m.title.as_str()
        ),
    );
    Ok(())
}

fn faction_remove(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let f = resolve_faction(ctx, ctx.args.lhs(), false)?;
    let rhs = rhs_required(ctx, "@faction/remove <faction>=<player>")?;
    let who = mush_core::match_object(ctx.w, ctx.actor, &rhs)?;
    let Some(removed) = state(ctx).members.remove(&(who, f.id)) else {
        return Err(CmdError::msg("Not a member of that faction."));
    };
    persist_del(ctx.out, member_key(who, f.id));
    // Undo only joins the module itself could have made: an eligible member
    // leaves the bound channel; a covert member's manual join is untouched.
    if channel_eligible(removed.visibility)
        && let Some((chan, _)) = bound_channel(ctx, f.id)
    {
        mush_comsys::api::force_leave(ctx, who, chan);
    }
    let name = display_name(ctx.w, who);
    info(ctx, format!("Removed {name} from {}.", f.name));
    Ok(())
}

fn faction_edge(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let from = resolve_faction(ctx, ctx.args.lhs(), false)?;
    let rhs = rhs_required(
        ctx,
        "@faction/edge <from>=<kind>/<to>[/vis:<v>][/note:<text>]",
    )?;
    let mut segs = rhs.split('/').map(str::trim);
    let kind = segs
        .next()
        .and_then(EdgeKind::parse)
        .ok_or_else(|| CmdError::msg("Edge kind: chapter, subordinate, contracted, or front."))?;
    let to_name = segs
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Name the target faction."))?;
    let to = resolve_faction(ctx, to_name, false)?;
    let mut vis = kind.default_visibility();
    let mut note = String::new();
    for seg in segs {
        match split_kv(seg) {
            Some((k, v))
                if k.eq_ignore_ascii_case("vis") || k.eq_ignore_ascii_case("visibility") =>
            {
                vis = Visibility::parse(v).ok_or_else(|| {
                    CmdError::msg("visibility: public, members, officers, or covert.")
                })?;
            }
            Some((k, v)) if k.eq_ignore_ascii_case("note") => note = v.to_string(),
            _ => return Err(CmdError::msg(format!("Bad modifier {seg:?}."))),
        }
    }
    if state_ref(ctx).would_cycle(from.id, kind, to.id) {
        return Err(CmdError::msg(format!(
            "That would make {} (transitively) {} itself — structural edges are acyclic per kind.",
            from.name,
            kind.label()
        )));
    }
    let updated = {
        let st = state(ctx);
        let fac = st.factions.get_mut(&from.id).expect("resolved above");
        fac.edges.retain(|e| !(e.kind == kind && e.to == to.id));
        fac.edges.push(FactionEdge {
            kind,
            to: to.id,
            visibility: vis,
            note,
        });
        fac.clone()
    };
    persist_faction(ctx.out, &updated);
    info(
        ctx,
        format!(
            "{} is now {} {} ({}).",
            updated.name,
            kind.label(),
            to.name,
            vis.label()
        ),
    );
    Ok(())
}

fn faction_unedge(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let from = resolve_faction(ctx, ctx.args.lhs(), false)?;
    let rhs = rhs_required(ctx, "@faction/unedge <from>=<kind>/<to>")?;
    let (kind_s, to_name) = rhs
        .split_once('/')
        .ok_or_else(|| CmdError::msg("Usage: @faction/unedge <from>=<kind>/<to>"))?;
    let kind = EdgeKind::parse(kind_s)
        .ok_or_else(|| CmdError::msg("Edge kind: chapter, subordinate, contracted, or front."))?;
    let to = resolve_faction(ctx, to_name, false)?;
    let (updated, removed) = {
        let st = state(ctx);
        let fac = st.factions.get_mut(&from.id).expect("resolved above");
        let before = fac.edges.len();
        fac.edges.retain(|e| !(e.kind == kind && e.to == to.id));
        (fac.clone(), fac.edges.len() != before)
    };
    if !removed {
        return Err(CmdError::msg("No such edge."));
    }
    persist_faction(ctx.out, &updated);
    info(
        ctx,
        format!(
            "{} is no longer {} {}.",
            updated.name,
            kind.label(),
            to.name
        ),
    );
    Ok(())
}

fn faction_principal(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let f = resolve_faction(ctx, ctx.args.lhs(), false)?;
    let rhs = rhs_required(ctx, "@faction/principal <faction>=<npc>  (toggles)")?;
    let npc = mush_core::match_object(ctx.w, ctx.actor, &rhs)?;
    if ctx.w.get(npc).is_none_or(|o| o.kind != ObjKind::Npc) {
        return Err(CmdError::msg("Faction principals are NPCs."));
    }
    let (updated, added) = {
        let st = state(ctx);
        let fac = st.factions.get_mut(&f.id).expect("resolved above");
        let added = if let Some(pos) = fac.principals.iter().position(|p| *p == npc) {
            fac.principals.remove(pos);
            false
        } else {
            fac.principals.push(npc);
            true
        };
        (fac.clone(), added)
    };
    persist_faction(ctx.out, &updated);
    let name = display_name(ctx.w, npc);
    info(
        ctx,
        if added {
            format!("{name} now personifies {}.", updated.name)
        } else {
            format!("{name} no longer personifies {}.", updated.name)
        },
    );
    Ok(())
}

fn faction_resource(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let f = resolve_faction(ctx, ctx.args.lhs(), false)?;
    let rhs = rhs_required(ctx, "@faction/resource <faction>=<kind>/<delta>[/<reason>]")?;
    let mut segs = rhs.splitn(3, '/').map(str::trim);
    let kind = segs
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Name a resource kind."))?;
    let delta: i64 = segs
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| CmdError::msg("Delta must be a signed integer."))?;
    let reason = segs.next().unwrap_or("staff adjustment").to_string();
    // E3d / D-E3-9: a migrated key is money, and money moves only through the
    // validated, conserved, reversible economy path. Letting this saturating
    // rating verb touch it would create value with no counterparty and no journal
    // entry — the exact thing the migration existed to end.
    let canonical = SmolStr::new(kind.to_ascii_lowercase());
    if f.migrated.contains(&canonical) {
        return Err(CmdError::msg(format!(
            "{canonical} is money on {}, not a rating: move it with @econ/pay, or \
             mint more with @econ/mint from a system source.",
            f.name
        )));
    }
    let actor = ctx.actor;
    let now = ctx.w.now();
    let updated = {
        let st = state(ctx);
        let fac = st.factions.get_mut(&f.id).expect("resolved above");
        fac.apply_resource(kind, delta, &reason, actor, now);
        fac.clone()
    };
    persist_faction(ctx.out, &updated);
    let total = updated
        .resources
        .get(&SmolStr::new(kind.to_ascii_lowercase()))
        .copied()
        .unwrap_or(0);
    info(
        ctx,
        format!(
            "{}: {} {delta:+} -> {total} ({reason}).",
            updated.name, kind
        ),
    );
    Ok(())
}

fn faction_channel(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let f = resolve_faction(ctx, ctx.args.lhs(), false)?;
    // `@faction/channel <f>` (no '=') INSPECTS; only the explicit `<f>=`
    // form unbinds — inspection must never be destructive.
    let Some(rhs) = ctx.args.rhs().map(|s| s.trim().to_string()) else {
        let bound = bound_channel(ctx, f.id)
            .and_then(|(chan, _)| mush_comsys::api::channel_name(ctx.w, chan));
        info(
            ctx,
            match bound {
                Some(name) => format!(
                    "{} is bound to channel {name}. (@faction/channel {}= unbinds.)",
                    f.name, f.name
                ),
                None => format!("{} has no bound channel.", f.name),
            },
        );
        return Ok(());
    };
    if rhs.is_empty() {
        let updated = {
            let st = state(ctx);
            let fac = st.factions.get_mut(&f.id).expect("resolved above");
            fac.channel = None;
            fac.clone()
        };
        persist_faction(ctx.out, &updated);
        info(
            ctx,
            format!(
                "Unbound {}'s channel (existing channel memberships are untouched).",
                updated.name
            ),
        );
        return Ok(());
    }
    let chan = mush_comsys::api::channel_id_by_name(ctx.w, &rhs)
        .ok_or_else(|| CmdError::msg("No such channel."))?;
    let (updated, eligible) = {
        let st = state(ctx);
        let fac = st.factions.get_mut(&f.id).expect("resolved above");
        fac.channel = Some(chan);
        let eligible: Vec<ObjId> = st
            .members_of(f.id)
            .filter(|m| channel_eligible(m.visibility))
            .map(|m| m.character)
            .collect();
        (st.factions.get(&f.id).expect("present").clone(), eligible)
    };
    persist_faction(ctx.out, &updated);
    let fname = updated.name.to_string();
    for who in &eligible {
        mush_comsys::api::force_join(ctx, *who, chan, &fname);
    }
    info(
        ctx,
        format!(
            "Bound {} to channel {rhs}; joined {} eligible (public) member{}.",
            updated.name,
            eligible.len(),
            if eligible.len() == 1 { "" } else { "s" }
        ),
    );
    // A comsys roster is readable by anyone passing the see lock, and the
    // join lock decides who may listen in — warn when they are wide open so
    // staff binding a "faction channel" know the roster and traffic are
    // public until they lock it.
    if let Some(perms) = mush_comsys::api::channel_perms(ctx.w, chan) {
        let open_see = perms.see == mush_core::LockExpr::Const(true);
        let open_join = perms.join == mush_core::LockExpr::Const(true);
        if open_see || open_join {
            info(
                ctx,
                format!(
                    "Note: {rhs}'s {} currently open — anyone can {}. \
                     Lock it with @channel/perm {rhs}=<see|join>=<lock>.",
                    match (open_see, open_join) {
                        (true, true) => "see and join locks are",
                        (true, false) => "see lock is",
                        _ => "join lock is",
                    },
                    match (open_see, open_join) {
                        (true, true) => "view its roster (@channel/who) and join it",
                        (true, false) => "view its roster (@channel/who)",
                        _ => "join it",
                    }
                ),
            );
        }
    }
    Ok(())
}

/// Parse `"<faction>/<kind>"` plus optional `/strength:<n>` / `/vis:<v>`
/// modifiers from a control spec.
fn parse_control_spec(
    ctx: &Ctx<'_>,
    rhs: &str,
) -> Result<(Faction, ControlKind, Option<u8>, Option<Visibility>), CmdError> {
    let mut segs = rhs.split('/').map(str::trim);
    let fac_name = segs
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Name the holding faction."))?;
    let faction = resolve_faction(ctx, fac_name, false)?;
    let kind = segs
        .next()
        .and_then(ControlKind::parse)
        .ok_or_else(|| CmdError::msg("Control kind: dejure, defacto, covert, or economic."))?;
    let (mut strength, mut vis) = (None, None);
    for seg in segs {
        match split_kv(seg) {
            Some((k, v)) if k.eq_ignore_ascii_case("strength") => {
                strength = Some(
                    v.parse::<u8>()
                        .map_err(|_| CmdError::msg("strength must be 0-255."))?,
                );
            }
            Some((k, v))
                if k.eq_ignore_ascii_case("vis") || k.eq_ignore_ascii_case("visibility") =>
            {
                vis = Some(Visibility::parse(v).ok_or_else(|| {
                    CmdError::msg("visibility: public, members, officers, or covert.")
                })?);
            }
            _ => return Err(CmdError::msg(format!("Bad modifier {seg:?}."))),
        }
    }
    Ok((faction, kind, strength, vis))
}

fn faction_control(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let region = resolve_region(ctx, ctx.args.lhs())?;
    let rhs = rhs_required(
        ctx,
        "@faction/control <region>=<faction>/<kind>[/strength:<n>][/vis:<v>]",
    )?;
    let (faction, kind, strength, vis) = parse_control_spec(ctx, &rhs)?;
    let now = ctx.w.now();
    let records = {
        let st = state(ctx);
        let recs = st.control.entry(region.id).or_default();
        if let Some(existing) = recs
            .iter_mut()
            .find(|r| r.faction == faction.id && r.kind == kind)
        {
            if let Some(s) = strength {
                existing.strength = s;
            }
            if let Some(v) = vis {
                existing.visibility = v;
            }
        } else {
            recs.push(ControlRecord {
                region: region.id,
                faction: faction.id,
                kind,
                strength: strength.unwrap_or(1),
                contested_by: Vec::new(),
                visibility: vis.unwrap_or_default(),
                since: now,
            });
        }
        recs.clone()
    };
    persist_control(ctx.out, region.id, &records);
    info(
        ctx,
        format!("{} holds {} ({}).", faction.name, region.name, kind.label()),
    );
    Ok(())
}

fn faction_uncontrol(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let region = resolve_region(ctx, ctx.args.lhs())?;
    let rhs = rhs_required(ctx, "@faction/uncontrol <region>=<faction>/<kind>")?;
    let (fac_name, kind_s) = rhs
        .split_once('/')
        .ok_or_else(|| CmdError::msg("Usage: @faction/uncontrol <region>=<faction>/<kind>"))?;
    let faction = resolve_faction(ctx, fac_name, false)?;
    let kind = ControlKind::parse(kind_s)
        .ok_or_else(|| CmdError::msg("Control kind: dejure, defacto, covert, or economic."))?;
    let (records, removed) = {
        let st = state(ctx);
        let Some(recs) = st.control.get_mut(&region.id) else {
            return Err(CmdError::msg("No control records on that region."));
        };
        let before = recs.len();
        recs.retain(|r| !(r.faction == faction.id && r.kind == kind));
        let removed = recs.len() != before;
        let snapshot = recs.clone();
        if snapshot.is_empty() {
            st.control.remove(&region.id);
        }
        (snapshot, removed)
    };
    if !removed {
        return Err(CmdError::msg("No such control record."));
    }
    persist_control(ctx.out, region.id, &records);
    info(
        ctx,
        format!(
            "{} no longer holds {} ({}).",
            faction.name,
            region.name,
            kind.label()
        ),
    );
    Ok(())
}

fn faction_contest(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let region = resolve_region(ctx, ctx.args.lhs())?;
    let rhs = rhs_required(
        ctx,
        "@faction/contest <region>=<holder>/<kind>/<challenger>  (toggles)",
    )?;
    let mut segs = rhs.splitn(3, '/').map(str::trim);
    let holder = resolve_faction(ctx, segs.next().unwrap_or(""), false)?;
    let kind = segs
        .next()
        .and_then(ControlKind::parse)
        .ok_or_else(|| CmdError::msg("Control kind: dejure, defacto, covert, or economic."))?;
    let challenger = resolve_faction(
        ctx,
        segs.next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| CmdError::msg("Name the contesting faction."))?,
        false,
    )?;
    let (records, now_contested) = {
        let st = state(ctx);
        let Some(recs) = st.control.get_mut(&region.id) else {
            return Err(CmdError::msg("No control records on that region."));
        };
        let Some(rec) = recs
            .iter_mut()
            .find(|r| r.faction == holder.id && r.kind == kind)
        else {
            return Err(CmdError::msg("No such control record."));
        };
        let now_contested =
            if let Some(pos) = rec.contested_by.iter().position(|c| *c == challenger.id) {
                rec.contested_by.remove(pos);
                false
            } else {
                rec.contested_by.push(challenger.id);
                true
            };
        (recs.clone(), now_contested)
    };
    persist_control(ctx.out, region.id, &records);
    info(
        ctx,
        if now_contested {
            format!(
                "{} now contests {}'s {} hold on {}.",
                challenger.name,
                holder.name,
                kind.label(),
                region.name
            )
        } else {
            format!(
                "{} no longer contests {}'s {} hold on {}.",
                challenger.name,
                holder.name,
                kind.label(),
                region.name
            )
        },
    );
    Ok(())
}

fn faction_show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let f = resolve_faction(ctx, ctx.args.raw(), false)?;
    let actor = ctx.actor;
    let st = state_ref(ctx);
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "Faction {} (id {}){}{}",
        f.name,
        f.id,
        if f.hidden { " [HIDDEN]" } else { "" },
        if f.aliases.is_empty() {
            String::new()
        } else {
            format!("  aka {}", f.aliases.join(", "))
        }
    ));
    if !f.charter.is_empty() {
        lines.push(format!("  Charter: {}", f.charter));
    }
    lines.push(format!(
        "  officer_level {}  intel_level {}  channel {}",
        f.officer_level,
        f.intel_level,
        f.channel.map_or("none".to_string(), |c| c.to_string())
    ));
    for e in &f.edges {
        let to = st
            .factions
            .get(&e.to)
            .map_or_else(|| format!("#{}", e.to), |t| t.name.to_string());
        lines.push(format!(
            "  Edge: {} {to} ({}){}",
            e.kind.label(),
            e.visibility.label(),
            if e.note.is_empty() {
                String::new()
            } else {
                format!(" — {}", e.note)
            }
        ));
    }
    for incident in &f.relay_condemnations {
        lines.push(format!(
            "  Condemns relay loss {}: responsible {:?}; {}",
            incident.object, incident.responsible, incident.source
        ));
    }
    for (kind, total) in &f.resources {
        lines.push(format!("  Resource: {kind} = {total}"));
    }
    // E3d: the backing-org link, resolved LIVE (so a missing org or a duplicate
    // binding is visible here rather than only at the moment money refuses), then
    // the org's root-scope balances behind the economy's own view gate.
    lines.push(format!("  Backing: {}", crate::econ::link_label(ctx.w, &f)));
    if !f.migrated.is_empty() {
        lines.push(format!(
            "  Migrated to money: {}",
            f.migrated
                .iter()
                .map(SmolStr::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    lines.extend(crate::econ::balance_lines(ctx, &f, "  "));
    for p in &f.principals {
        lines.push(format!("  Principal NPC: {}", display_name(ctx.w, *p)));
    }
    let mut members: Vec<&FactionMembership> = st.members_of(f.id).collect();
    members.sort_by_key(|m| std::cmp::Reverse(m.level));
    lines.push(format!("  Members ({}):", members.len()));
    for m in members {
        lines.push(format!(
            "    {} — level {} ({}){}",
            display_name(ctx.w, m.character),
            m.level,
            m.visibility.label(),
            if m.title.is_empty() {
                String::new()
            } else {
                format!(", {}", m.title)
            }
        ));
    }
    let holdings: Vec<String> = st
        .control
        .iter()
        .flat_map(|(region, recs)| {
            let region_name = st
                .regions
                .get(region)
                .map_or_else(|| format!("#{region}"), |r| r.name.to_string());
            recs.iter()
                .filter(|r| r.faction == f.id)
                .map(move |r| {
                    format!(
                        "    {} ({}, strength {}, {})",
                        region_name.clone(),
                        r.kind.label(),
                        r.strength,
                        r.visibility.label()
                    )
                })
                .collect::<Vec<_>>()
        })
        .collect();
    if !holdings.is_empty() {
        lines.push("  Holdings:".to_string());
        lines.extend(holdings);
    }
    for line in lines {
        ctx.out.sys_to_actor(actor, "faction", line);
    }
    Ok(())
}

fn faction_staff_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let st = state_ref(ctx);
    if st.factions.is_empty() {
        ctx.out.sys_to_actor(
            actor,
            "faction",
            "No factions exist. (@faction/create <name>)",
        );
        return Ok(());
    }
    let lines: Vec<String> = st
        .factions
        .values()
        .map(|f| {
            format!(
                "  {:>3}  {}{}  ({} member{})",
                f.id.to_string(),
                f.name,
                if f.hidden { " [HIDDEN]" } else { "" },
                st.members_of(f.id).count(),
                if st.members_of(f.id).count() == 1 {
                    ""
                } else {
                    "s"
                }
            )
        })
        .collect();
    ctx.out.sys_to_actor(actor, "faction", "Factions:");
    for l in lines {
        ctx.out.sys_to_actor(actor, "faction", l);
    }
    Ok(())
}

// ----------------------------------------------------------------------
// @region — staff region administration
// ----------------------------------------------------------------------

fn cmd_region(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_admin(ctx)?;
    if ctx.switches.has("create") {
        region_create(ctx)
    } else if ctx.switches.has("destroy") {
        region_destroy(ctx)
    } else if ctx.switches.has("zone") {
        region_zone(ctx)
    } else if ctx.switches.has("atlas") {
        region_atlas(ctx)
    } else if ctx.switches.has("note") {
        region_note(ctx)
    } else {
        region_list(ctx)
    }
}

/// `@region/atlas <region>=<place>` (S2b): point a control region at an atlas
/// place — a neighborhood, a locality or a region, by name or id — or clear it
/// with an empty right-hand side. The place's footprint is what `+territory`
/// shows for it.
fn region_atlas(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let r = resolve_region(ctx, ctx.args.lhs())?;
    let rhs = ctx.args.rhs().unwrap_or("").trim().to_string();
    let place = if rhs.is_empty() {
        None
    } else {
        let records = mush_atlas::place::records(ctx.w)
            .ok_or_else(|| CmdError::msg("The atlas is not composed on this server."))?;
        match mush_atlas::place::find_place(records, &rhs) {
            Some(mush_atlas::PlaceRef::World) | Some(mush_atlas::PlaceRef::Site(_)) => {
                return Err(CmdError::msg(
                    "A control region stands on a neighborhood, a locality or a region.",
                ));
            }
            Some(place) => Some((place.id(), place.name(records))),
            None => {
                return Err(CmdError::msg(format!(
                    "No neighborhood, locality or region called {rhs:?} on the atlas."
                )));
            }
        }
    };
    let updated = {
        let st = state(ctx);
        let Some(region) = st.regions.get_mut(&r.id) else {
            return Err(CmdError::msg("That region is gone."));
        };
        region.atlas = place.as_ref().map(|(id, _)| id.clone());
        region.clone()
    };
    persist_region(ctx.out, &updated);
    info(
        ctx,
        match place {
            Some((id, name)) => format!("Region {} now stands on {name} ({id}).", updated.name),
            None => format!("Region {} stands on no atlas place.", updated.name),
        },
    );
    Ok(())
}

fn region_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let name = ctx.args.lhs().trim().to_string();
    if !valid_name(&name) {
        return Err(CmdError::msg(format!(
            "That is not a usable region name (1-{MAX_NAME_LEN} letters, digits, space, - _ ' .)."
        )));
    }
    if state_ref(ctx).region_by_name(&name).is_some() {
        return Err(CmdError::msg("A region by that name already exists."));
    }
    let (r, next_f, next_r) = {
        let st = state(ctx);
        let id = st.alloc_region_id();
        let r = Region {
            id,
            name: SmolStr::new(&name),
            note: String::new(),
            zones: Vec::new(),
            atlas: None,
        };
        st.regions.insert(id, r.clone());
        (r, st.next_faction_id, st.next_region_id)
    };
    persist_region(ctx.out, &r);
    persist_meta(ctx.out, next_f, next_r);
    info(ctx, format!("Created region {name} (id {}).", r.id));
    Ok(())
}

fn region_destroy(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let r = resolve_region(ctx, ctx.args.lhs())?;
    {
        let st = state(ctx);
        st.regions.remove(&r.id);
        st.control.remove(&r.id);
    }
    persist_del(ctx.out, region_key(r.id));
    persist_del(ctx.out, control_key(r.id));
    // Detach any clock consequence that would flip this now-gone region.
    let changed = ctx
        .w
        .module_state_mut::<crate::SimState>()
        .map(|s| crate::sim::detach_region(s, r.id))
        .unwrap_or_default();
    for c in &changed {
        crate::sim::persist_clock(ctx.out, c);
    }
    info(ctx, format!("Destroyed region {}.", r.name));
    Ok(())
}

fn region_zone(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let r = resolve_region(ctx, ctx.args.lhs())?;
    let rhs = rhs_required(ctx, "@region/zone <region>=<zone master object>  (toggles)")?;
    let zmo = mush_core::match_object(ctx.w, ctx.actor, &rhs)?;
    if ctx
        .w
        .get(zmo)
        .is_none_or(|o| !o.flags.contains(Flags::ZONE_MASTER))
    {
        return Err(CmdError::msg("That is not a zone master object."));
    }
    let (updated, added) = {
        let st = state(ctx);
        let region = st.regions.get_mut(&r.id).expect("resolved above");
        let added = if let Some(pos) = region.zones.iter().position(|z| *z == zmo) {
            region.zones.remove(pos);
            false
        } else {
            region.zones.push(zmo);
            true
        };
        (region.clone(), added)
    };
    persist_region(ctx.out, &updated);
    let zname = display_name(ctx.w, zmo);
    info(
        ctx,
        if added {
            format!("Region {} now covers zone {zname}.", updated.name)
        } else {
            format!("Region {} no longer covers zone {zname}.", updated.name)
        },
    );
    Ok(())
}

fn region_note(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let r = resolve_region(ctx, ctx.args.lhs())?;
    let note = ctx.args.rhs().unwrap_or("").trim().to_string();
    let updated = {
        let st = state(ctx);
        let region = st.regions.get_mut(&r.id).expect("resolved above");
        region.note = note;
        region.clone()
    };
    persist_region(ctx.out, &updated);
    info(ctx, format!("Note set on region {}.", updated.name));
    Ok(())
}

fn region_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let st = state_ref(ctx);
    if st.regions.is_empty() {
        ctx.out.sys_to_actor(
            actor,
            "faction",
            "No regions exist. (@region/create <name>)",
        );
        return Ok(());
    }
    let lines: Vec<String> = st
        .regions
        .values()
        .map(|r| {
            let zones: Vec<String> = r.zones.iter().map(|z| display_name(ctx.w, *z)).collect();
            format!(
                "  {:>3}  {}  ({}){}",
                r.id.to_string(),
                r.name,
                if zones.is_empty() {
                    "non-spatial".to_string()
                } else {
                    zones.join(", ")
                },
                if r.note.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", r.note)
                }
            )
        })
        .collect();
    ctx.out.sys_to_actor(actor, "faction", "Regions:");
    for l in lines {
        ctx.out.sys_to_actor(actor, "faction", l);
    }
    Ok(())
}

// ----------------------------------------------------------------------
// Player surface: +factions / +faction / +territory
// ----------------------------------------------------------------------

fn cmd_factions(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let staff = is_admin(ctx.w, actor);
    let st = state_ref(ctx);
    let listed: Vec<String> = st
        .factions
        .values()
        .filter(|f| st.faction_listed(actor, staff, f))
        .map(|f| {
            let visible = st
                .members_of(f.id)
                .filter(|m| st.membership_visible(actor, staff, m))
                .count();
            let mine = st.membership(actor, f.id).map(|m| {
                if m.title.is_empty() {
                    " — you are a member".to_string()
                } else {
                    format!(" — you are {}", m.title)
                }
            });
            format!(
                "  {}{}  ({visible} known member{}){}",
                f.name,
                if staff && f.hidden { " [HIDDEN]" } else { "" },
                if visible == 1 { "" } else { "s" },
                mine.unwrap_or_default()
            )
        })
        .collect();
    if listed.is_empty() {
        ctx.out
            .sys_to_actor(actor, "faction", "No factions are known.");
        return Ok(());
    }
    ctx.out.sys_to_actor(actor, "faction", "Factions:");
    for l in listed {
        ctx.out.sys_to_actor(actor, "faction", l);
    }
    Ok(())
}

fn cmd_faction_info(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let arg = ctx.args.raw().trim().to_string();
    if arg.is_empty() {
        return cmd_factions(ctx);
    }
    let f = resolve_faction(ctx, &arg, true)?;
    let actor = ctx.actor;
    let staff = is_admin(ctx.w, actor);
    let st = state_ref(ctx);
    let is_member = st.is_member(actor, f.id);
    let is_officer = st.is_officer(actor, f.id);

    let mut lines: Vec<String> = Vec::new();
    lines.push(format!(
        "{}{}",
        f.name,
        if f.aliases.is_empty() {
            String::new()
        } else {
            format!("  (also known as {})", f.aliases.join(", "))
        }
    ));
    if !f.charter.is_empty() {
        lines.push(format!("  {}", f.charter));
    }
    // Structure — an edge shows only if the edge's visibility passes AND the
    // target faction is itself listable to this viewer (a visible edge must
    // not reveal a hidden faction's existence).
    for e in &f.edges {
        if !st.visibility_passes(actor, staff, f.id, e.visibility) {
            continue;
        }
        let Some(target) = st.factions.get(&e.to) else {
            continue;
        };
        if !st.faction_listed(actor, staff, target) {
            continue;
        }
        lines.push(format!("  {} {}.", cap_first(e.kind.label()), target.name));
    }
    // Roster — visibility-projected; levels are officer/staff knowledge.
    let mut members: Vec<&FactionMembership> = st
        .members_of(f.id)
        .filter(|m| st.membership_visible(actor, staff, m))
        .collect();
    // Rank order is itself officer/staff knowledge: a level-sorted roster
    // would leak the ranking even with the "[level N]" labels hidden, so
    // everyone else gets an alphabetical roster.
    if is_officer || staff {
        members.sort_by_key(|m| std::cmp::Reverse(m.level));
    } else {
        members.sort_by_key(|m| display_name(ctx.w, m.character).to_lowercase());
    }
    if !members.is_empty() {
        lines.push(format!("  Known members ({}):", members.len()));
        for m in members {
            let level = if is_officer || staff {
                format!(" [level {}]", m.level)
            } else {
                String::new()
            };
            lines.push(format!(
                "    {}{}{}",
                display_name(ctx.w, m.character),
                if m.title.is_empty() {
                    String::new()
                } else {
                    format!(", {}", m.title)
                },
                level
            ));
        }
    }
    // Principals and resources are internal knowledge.
    if is_member || staff {
        for p in &f.principals {
            lines.push(format!(
                "  Voice of the faction: {}",
                display_name(ctx.w, *p)
            ));
        }
    }
    if is_officer || staff {
        for incident in &f.relay_condemnations {
            lines.push(format!(
                "  Condemns relay loss {}: responsible {:?}; {}",
                incident.object, incident.responsible, incident.source
            ));
        }
        for (kind, total) in &f.resources {
            lines.push(format!("  Resource: {kind} = {total}"));
        }
    }
    // Money is NOT internal faction knowledge (§6.2, D-E3-11). The officer gate
    // above governs ratings; the backing org's balances answer to the economy's own
    // §4.5 view rule — staff, or `economy.account.view` on the org's root scope — so
    // a faction officer with no `OrgMembership` and no grant sees ratings here and
    // no money, and there is no "hidden" marker to tell them a wallet exists.
    lines.extend(crate::econ::balance_lines(ctx, &f, "  "));
    // Holdings — control visibility, with the holder's officers always aware
    // of their own faction's holds (covert or not).
    let holdings: Vec<String> = st
        .control
        .iter()
        .flat_map(|(region, recs)| {
            recs.iter()
                .filter(|r| r.faction == f.id)
                .filter(|r| {
                    st.visibility_passes(actor, staff, r.faction, r.visibility)
                        || st.is_officer(actor, r.faction)
                })
                .map(|r| {
                    let region_name = st
                        .regions
                        .get(region)
                        .map_or_else(|| format!("#{region}"), |x| x.name.to_string());
                    format!("  Holds {} ({}).", region_name, r.kind.label())
                })
                .collect::<Vec<_>>()
        })
        .collect();
    lines.extend(holdings);
    if let Some(m) = st.membership(actor, f.id) {
        lines.push(format!(
            "  Your membership: level {}{}{}",
            m.level,
            if m.title.is_empty() {
                String::new()
            } else {
                format!(", {}", m.title)
            },
            if matches!(m.visibility, Visibility::Covert) {
                "  (covert — only you and staff know)"
            } else {
                ""
            }
        ));
    }
    let _ = is_member;
    for line in lines {
        ctx.out.sys_to_actor(actor, "faction", line);
    }
    Ok(())
}

fn cap_first(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn cmd_territory(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let staff = is_admin(ctx.w, actor);
    let st = state_ref(ctx);
    let filter = ctx.args.raw().trim().to_lowercase();
    let mut regions: Vec<&Region> = st.regions.values().collect();
    regions.sort_by(|a, b| a.name.cmp(&b.name));
    let mut lines: Vec<String> = Vec::new();
    for r in regions {
        if !filter.is_empty() && !r.name.to_lowercase().contains(&filter) {
            continue;
        }
        let holds: Vec<String> = st
            .control
            .get(&r.id)
            .map(|recs| {
                recs.iter()
                    .filter(|rec| {
                        st.visibility_passes(actor, staff, rec.faction, rec.visibility)
                            || st.is_officer(actor, rec.faction)
                    })
                    .filter_map(|rec| {
                        let holder = st.factions.get(&rec.faction)?;
                        if !st.faction_listed(actor, staff, holder) {
                            return None;
                        }
                        let contested: Vec<String> = rec
                            .contested_by
                            .iter()
                            .filter_map(|c| st.factions.get(c))
                            .filter(|c| st.faction_listed(actor, staff, c))
                            .map(|c| c.name.to_string())
                            .collect();
                        Some(format!(
                            "{} ({}{})",
                            holder.name,
                            rec.kind.label(),
                            if contested.is_empty() {
                                String::new()
                            } else {
                                format!(", contested by {}", contested.join(", "))
                            }
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let hold_text = if holds.is_empty() {
            "unclaimed".to_string()
        } else {
            holds.join("; ")
        };
        // S2b: a region that stands on an atlas place shows its footprint.
        let footprint = r
            .atlas
            .as_deref()
            .and_then(|id| {
                let records = mush_atlas::place::records(ctx.w)?;
                let place = mush_atlas::place::find_place(records, id)?;
                let (w_m, h_m, x0, y0) = place.extent_m(records)?;
                Some(format!(
                    "  [map: {} {}, {:.0} m × {:.0} m at ({:.0}, {:.0})]",
                    place.kind(),
                    place.name(records),
                    w_m,
                    h_m,
                    x0,
                    y0
                ))
            })
            .unwrap_or_default();
        lines.push(format!("  {} — {hold_text}{footprint}", r.name));
    }
    if lines.is_empty() {
        ctx.out
            .sys_to_actor(actor, "faction", "No territory is mapped.");
        return Ok(());
    }
    ctx.out.sys_to_actor(actor, "faction", "Territory:");
    for l in lines {
        ctx.out.sys_to_actor(actor, "faction", l);
    }
    Ok(())
}

// ----------------------------------------------------------------------
// Registration
// ----------------------------------------------------------------------

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@faction".into(),
            shape: ArgShape::EqPair,
            switches: vec![
                "create".into(),
                "destroy".into(),
                "desc".into(),
                "set".into(),
                "add".into(),
                "mod".into(),
                "remove".into(),
                "edge".into(),
                "unedge".into(),
                "principal".into(),
                "resource".into(),
                "org".into(),
                "migrate".into(),
                "confirm".into(),
                "channel".into(),
                "control".into(),
                "uncontrol".into(),
                "contest".into(),
                "incident".into(),
                "publish-incident".into(),
                "show".into(),
            ],
            help: "@faction  --  Staff faction registry (Royalty+ or the FACTION_ADMIN power). \
                   No switch: list all factions.\n\
                   @faction/create <name> ; @faction/destroy <name> ; @faction/desc <f>=<charter>\n\
                   @faction/set <f>=<field>:<value>  (hidden, officer, intel, aliases, ext)\n\
                   @faction/add <f>=<player>[/level:<n>][/vis:<v>][/title:<t>] ; \
                   @faction/mod likewise ; @faction/remove <f>=<player>\n\
                   Visibilities: public, members, officers, covert -- the membership FACT is \
                   engine truth used for authorization; who can SEE it is the visibility policy.\n\
                   @faction/edge <from>=<kind>/<to>[/vis:<v>][/note:<text>]  (kinds: chapter, \
                   subordinate, contracted, front; front edges default covert; acyclic per kind) ; \
                   @faction/unedge <from>=<kind>/<to>\n\
                   @faction/principal <f>=<npc>  (toggle the NPC voice of the faction)\n\
                   @faction/resource <f>=<kind>/<delta>[/<reason>]  (typed RATING pools, \
                   cause-stamped; a migrated key is money and is refused here)\n\
                   @faction/org <f>=<org|new|none>  (bind the faction's money identity; new \
                   mints an org + root scope; none unbinds and leaves the org and its accounts \
                   standing. A bound faction cannot be destroyed.)\n\
                   @faction/migrate <f>=<kind> <system account> [/confirm]  (ONE key per call: \
                   the rating integer becomes that instrument's MINOR UNITS, verbatim, minted \
                   from the named system source into the org's root-scope account. One way, \
                   once per key; without /confirm it only shows what would move.)\n\
                   @faction/channel <f>  (show binding) ; @faction/channel <f>=<channel>  (bind; \
                   PUBLIC members auto-join — quieter visibilities never ride the roster; \
                   @faction/channel <f>= with an explicit '=' unbinds; lock the channel's \
                   see/join or the roster and traffic are world-open)\n\
                   @faction/control <region>=<f>/<kind>[/strength:<n>][/vis:<v>]  (kinds: dejure, \
                   defacto, covert, economic) ; @faction/uncontrol <region>=<f>/<kind> ; \
                   @faction/contest <region>=<holder>/<kind>/<challenger>  (toggle)\n\
                   @faction/incident <cause>=<f>/<victim #id>/<attacker #id|unknown>/<reason> \
                   records a PRIVATE adjudicated report, with no standing change. Without '=', \
                   inspect the report. @faction/publish-incident <cause> explicitly publishes \
                   it and applies -20 standing to an identified attacker, once. Ordinary combat \
                   and unverified rumors do not publish incidents automatically.\n\
                   @faction/show <f>  --  full staff dump (truth, all visibilities)."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_faction),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::ReadOnly);
    reg.command(
        CommandSpec {
            name: "+standing".into(),
            shape: ArgShape::Raw,
            help: "+standing [<faction>]  --  Your standing with public factions, from verified \
                   work and explicitly published incidents. Name one faction for the last ten \
                   causes. Scores range from -100 to 100. Standing does not grant membership, \
                   rank, intelligence clearance, or spending authority."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(crate::reputation::command_standing),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    reg.command(
        CommandSpec {
            name: "@region".into(),
            shape: ArgShape::EqPair,
            switches: vec![
                "create".into(),
                "destroy".into(),
                "zone".into(),
                "atlas".into(),
                "note".into(),
            ],
            help: "@region  --  Staff territory regions (Royalty+ or FACTION_ADMIN). No switch: \
                   list. A region is deliberately NOT the room graph: it may cover zone master \
                   objects, stand on an atlas place, or nothing at all (a non-spatial region \
                   like \"Media influence\").\n\
                   @region/create <name> ; @region/destroy <name>\n\
                   @region/zone <region>=<zone master object>  (toggle coverage)\n\
                   @region/atlas <region>=<neighborhood|locality|region>  (its footprint shows \
                   in +territory; empty clears)\n\
                   @region/note <region>=<text>"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_region),
    )?;
    reg.command(
        CommandSpec {
            name: "+factions".into(),
            shape: ArgShape::None,
            help: "+factions  --  List the factions you know of, with their publicly known \
                   member counts. Hidden factions appear only to their members."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_factions),
    )?;
    reg.command(
        CommandSpec {
            name: "+faction".into(),
            shape: ArgShape::Raw,
            help: "+faction <name>  --  What you know about one faction: charter, known \
                   structure, known members (titles; officers also see levels), known holdings, \
                   and your own membership if any. What you see is a projection — covert \
                   memberships, covert holdings, and hidden factions do not appear."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_faction_info),
    )?;
    reg.command(
        CommandSpec {
            name: "+territory".into(),
            shape: ArgShape::Raw,
            help: "+territory [<region>]  --  The territory map as you know it: each region and \
                   who is known to hold it (de jure claims, de facto holds, contests). Covert \
                   holds show only to the holder's officers and staff."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_territory),
    )?;
    Ok(())
}

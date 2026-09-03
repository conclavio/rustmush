//! Faction-simulation commands: `@front` and `@clock` (staff) plus `+clocks`
//! (the player view of visible faction pressure).

use std::sync::Arc;

use mush_core::{
    ArgShape, CmdError, CommandSpec, Ctx, ModError, ObjId, Registrar, Role, World, perm,
};
use smol_str::SmolStr;

use crate::data::{ControlKind, FactionState, Visibility};
use crate::sim::{
    Clock, ClockId, Front, FrontId, SimState, clock_key, persist_clock, persist_del, persist_front,
    persist_sim_meta,
};

fn sim<'a>(ctx: &'a mut Ctx<'_>) -> &'a mut SimState {
    ctx.w
        .module_state_mut::<SimState>()
        .expect("sim state slot installed at registration")
}

fn sim_ref<'a>(ctx: &'a Ctx<'_>) -> &'a SimState {
    ctx.w
        .module_state::<SimState>()
        .expect("sim state slot installed at registration")
}

fn fac_ref<'a>(ctx: &'a Ctx<'_>) -> &'a FactionState {
    ctx.w
        .module_state::<FactionState>()
        .expect("faction state slot installed at registration")
}

fn is_admin(w: &World, who: ObjId) -> bool {
    perm::has_power(w, who, mush_core::Powers::FACTION_ADMIN)
        || perm::has_role(w, who, Role::Royalty)
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
    ctx.out.sys_to_actor(actor, "faction", msg.into());
}

// ----------------------------------------------------------------------
// @front
// ----------------------------------------------------------------------

fn cmd_front(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_admin(ctx)?;
    if ctx.switches.has("create") {
        front_create(ctx)
    } else if ctx.switches.has("resolve") {
        front_resolve(ctx)
    } else {
        front_list(ctx)
    }
}

fn front_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let fac_name = ctx.args.lhs().trim().to_string();
    let title = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| CmdError::msg("Usage: @front/create <faction>=<title>"))?
        .to_string();
    let faction = fac_ref(ctx)
        .faction_by_name(&fac_name)
        .map(|f| f.id)
        .ok_or_else(|| CmdError::msg("No such faction."))?;
    let (front, next_f, next_c) = {
        let st = sim(ctx);
        let id = st.alloc_front_id();
        let front = Front {
            id,
            faction,
            title,
            resolved: false,
        };
        st.fronts.insert(id, front.clone());
        (front, st.next_front_id, st.next_clock_id)
    };
    persist_front(ctx.out, &front);
    persist_sim_meta_now(ctx, next_f, next_c);
    info(
        ctx,
        format!("Created front #{} ({}).", front.id, front.title),
    );
    Ok(())
}

fn front_resolve(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_id(ctx.args.lhs())
        .map(FrontId)
        .ok_or_else(|| CmdError::msg("Usage: @front/resolve <id>"))?;
    let updated = {
        let st = sim(ctx);
        let Some(front) = st.fronts.get_mut(&id) else {
            return Err(CmdError::msg("No such front."));
        };
        front.resolved = true;
        front.clone()
    };
    persist_front(ctx.out, &updated);
    info(ctx, format!("Front #{} marked resolved.", id));
    Ok(())
}

fn front_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lines: Vec<String> = {
        let st = sim_ref(ctx);
        let fac = fac_ref(ctx);
        st.fronts
            .values()
            .map(|f| {
                let fname = fac
                    .factions
                    .get(&f.faction)
                    .map_or_else(|| format!("#{}", f.faction), |x| x.name.to_string());
                let n = st.clocks.values().filter(|c| c.front == f.id).count();
                format!(
                    "  #{}  {} [{}]{}  ({n} clock{})",
                    f.id,
                    f.title,
                    fname,
                    if f.resolved { " RESOLVED" } else { "" },
                    if n == 1 { "" } else { "s" }
                )
            })
            .collect()
    };
    if lines.is_empty() {
        info(ctx, "No fronts. (@front/create <faction>=<title>)");
        return Ok(());
    }
    info(ctx, "Fronts:");
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

// ----------------------------------------------------------------------
// @clock
// ----------------------------------------------------------------------

fn cmd_clock(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    require_admin(ctx)?;
    if ctx.switches.has("create") {
        clock_create(ctx)
    } else if ctx.switches.has("set") {
        clock_set(ctx)
    } else if ctx.switches.has("advance") {
        clock_advance_cmd(ctx)
    } else if ctx.switches.has("del") {
        clock_del(ctx)
    } else {
        clock_list(ctx)
    }
}

fn clock_create(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let front_id = parse_id(ctx.args.lhs())
        .map(FrontId)
        .ok_or_else(|| CmdError::msg("Usage: @clock/create <front-id>=<title>/<segments>"))?;
    let rhs = ctx
        .args
        .rhs()
        .ok_or_else(|| CmdError::msg("Usage: @clock/create <front-id>=<title>/<segments>"))?;
    let (title, seg_s) = rhs
        .rsplit_once('/')
        .ok_or_else(|| CmdError::msg("Usage: @clock/create <front-id>=<title>/<segments>"))?;
    let segments: u8 = seg_s
        .trim()
        .parse()
        .ok()
        .filter(|&n| (1..=20).contains(&n))
        .ok_or_else(|| CmdError::msg("Segments must be 1-20."))?;
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err(CmdError::msg("Give the clock a title."));
    }
    let faction = {
        let st = sim_ref(ctx);
        st.fronts
            .get(&front_id)
            .map(|f| f.faction)
            .ok_or_else(|| CmdError::msg("No such front."))?
    };
    let (clock, next_f, next_c) = {
        let st = sim(ctx);
        let id = st.alloc_clock_id();
        let clock = Clock {
            id,
            front: front_id,
            faction,
            title,
            segments,
            filled: 0,
            visibility: Visibility::OfficersKnow,
            advance_on: None,
            consequence: String::new(),
            on_complete_control: None,
            completed: false,
        };
        st.clocks.insert(id, clock.clone());
        (clock, st.next_front_id, st.next_clock_id)
    };
    persist_clock(ctx.out, &clock);
    persist_sim_meta_now(ctx, next_f, next_c);
    info(
        ctx,
        format!(
            "Created clock #{} ({}) on front #{}, {}. Configure with @clock/set.",
            clock.id,
            clock.title,
            clock.front,
            clock.progress()
        ),
    );
    Ok(())
}

fn clock_set(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_id(ctx.args.lhs())
        .map(ClockId)
        .ok_or_else(|| CmdError::msg("Usage: @clock/set <id>=<field>:<value>"))?;
    let rhs = ctx
        .args
        .rhs()
        .and_then(|s| s.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .ok_or_else(|| {
            CmdError::msg(
                "Usage: @clock/set <id>=<field>:<value>  (fields: on, vis, says, control, unset)",
            )
        })?;
    let (field, value) = rhs;
    // Resolve a control region up front (needs FactionState, before the sim borrow).
    let control = if field == "control" {
        Some(parse_control(ctx, &value)?)
    } else {
        None
    };
    let updated = {
        let st = sim(ctx);
        let Some(clock) = st.clocks.get_mut(&id) else {
            return Err(CmdError::msg("No such clock."));
        };
        match field.as_str() {
            "on" => {
                if value == crate::CONSEQUENCE_KIND {
                    return Err(CmdError::msg(
                        "That kind is reserved for clock consequences; a clock cannot watch it \
                         (it would let clocks cascade into each other).",
                    ));
                }
                clock.advance_on = if value.is_empty() {
                    None
                } else {
                    Some(SmolStr::new(&value))
                };
            }
            "vis" | "visibility" => {
                clock.visibility = Visibility::parse(&value)
                    .ok_or_else(|| CmdError::msg("vis: public, members, officers, or covert."))?;
            }
            "says" | "consequence" => clock.consequence = value.clone(),
            "control" => clock.on_complete_control = control,
            "unset" if value.eq_ignore_ascii_case("control") => clock.on_complete_control = None,
            _ => {
                return Err(CmdError::msg(
                    "Unknown field. Fields: on (canon kind), vis, says, control (<region>/<kind>).",
                ));
            }
        }
        clock.clone()
    };
    persist_clock(ctx.out, &updated);
    info(ctx, format!("Updated clock #{}.", updated.id));
    Ok(())
}

/// Parse a `control` value `<region>/<kind>` into `(RegionId, ControlKind)`.
fn parse_control(
    ctx: &Ctx<'_>,
    value: &str,
) -> Result<(crate::data::RegionId, ControlKind), CmdError> {
    let (region_name, kind_s) = value.rsplit_once('/').ok_or_else(|| {
        CmdError::msg("control: <region>/<kind> (kind: dejure/defacto/covert/economic).")
    })?;
    let region = fac_ref(ctx)
        .region_by_name(region_name.trim())
        .map(|r| r.id)
        .ok_or_else(|| CmdError::msg("No such region."))?;
    let kind = ControlKind::parse(kind_s)
        .ok_or_else(|| CmdError::msg("control kind: dejure, defacto, covert, or economic."))?;
    Ok((region, kind))
}

fn clock_advance_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_id(ctx.args.lhs())
        .map(ClockId)
        .ok_or_else(|| CmdError::msg("Usage: @clock/advance <id>[=<n>]"))?;
    let n: u8 = ctx
        .args
        .rhs()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map_or(Ok(1), |s| {
            s.parse()
                .map_err(|_| CmdError::msg("Amount must be a number."))
        })?;
    if !sim_ref(ctx).clocks.contains_key(&id) {
        return Err(CmdError::msg("No such clock."));
    }
    let completed = crate::advance_clock(ctx, id, n);
    let progress = sim_ref(ctx)
        .clocks
        .get(&id)
        .map_or_else(String::new, Clock::progress);
    info(
        ctx,
        if completed {
            format!("Clock #{id} advanced to {progress} — COMPLETE; its consequence has fired.")
        } else {
            format!("Clock #{id} advanced to {progress}.")
        },
    );
    Ok(())
}

fn clock_del(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let id = parse_id(ctx.args.lhs())
        .map(ClockId)
        .ok_or_else(|| CmdError::msg("Usage: @clock/del <id>"))?;
    if sim(ctx).clocks.remove(&id).is_none() {
        return Err(CmdError::msg("No such clock."));
    }
    persist_del(ctx.out, clock_key(id));
    info(ctx, format!("Deleted clock #{id}."));
    Ok(())
}

fn clock_list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lines: Vec<String> = {
        let st = sim_ref(ctx);
        st.clocks
            .values()
            .map(|c| {
                let watch = c
                    .advance_on
                    .as_deref()
                    .map_or_else(String::new, |k| format!("  on:{k}"));
                format!(
                    "  #{}  {} [{}] front#{} <{}>{}{}",
                    c.id,
                    c.title,
                    c.progress(),
                    c.front,
                    c.visibility.label(),
                    watch,
                    if c.completed { "  DONE" } else { "" }
                )
            })
            .collect()
    };
    if lines.is_empty() {
        info(
            ctx,
            "No clocks. (@clock/create <front-id>=<title>/<segments>)",
        );
        return Ok(());
    }
    info(ctx, "Clocks:");
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

// ----------------------------------------------------------------------
// +clocks (player view)
// ----------------------------------------------------------------------

fn cmd_player_clocks(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let actor = ctx.actor;
    let staff = is_admin(ctx.w, actor);
    let lines: Vec<String> = {
        let st = sim_ref(ctx);
        let fac = fac_ref(ctx);
        st.clocks
            .values()
            .filter(|c| fac.visibility_passes(actor, staff, c.faction, c.visibility))
            .map(|c| {
                let fname = fac
                    .factions
                    .get(&c.faction)
                    .map_or_else(|| "?".to_string(), |x| x.name.to_string());
                format!("  {} [{}] — {}", c.title, c.progress(), fname)
            })
            .collect()
    };
    if lines.is_empty() {
        info(ctx, "You know of no active faction clocks.");
        return Ok(());
    }
    info(ctx, "Faction clocks you know of:");
    for l in lines {
        info(ctx, l);
    }
    Ok(())
}

// ----------------------------------------------------------------------
// helpers + registration
// ----------------------------------------------------------------------

fn parse_id(s: &str) -> Option<u64> {
    s.trim().trim_start_matches('#').parse().ok()
}

fn persist_sim_meta_now(ctx: &mut Ctx<'_>, next_front: u64, next_clock: u64) {
    let st = SimState {
        next_front_id: next_front,
        next_clock_id: next_clock,
        ..SimState::default()
    };
    persist_sim_meta(ctx.out, &st);
}

pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "@front".into(),
            shape: ArgShape::EqPair,
            switches: vec!["create".into(), "resolve".into()],
            help: "@front  --  (staff) Faction fronts: active threats/opportunities a faction \
                   pursues. No switch: list.\n\
                   @front/create <faction>=<title> ; @front/resolve <id>"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_front),
    )?;
    reg.command(
        CommandSpec {
            name: "@clock".into(),
            shape: ArgShape::EqPair,
            switches: vec![
                "create".into(),
                "set".into(),
                "advance".into(),
                "del".into(),
            ],
            help: "@clock  --  (staff) Progress clocks on fronts. No switch: list.\n\
                   @clock/create <front-id>=<title>/<segments>\n\
                   @clock/set <id>=<field>:<value>  (on:<canon kind> auto-advances it when such \
                   canon is recorded involving the faction's members; vis:<v>; says:<consequence \
                   line>; control:<region>/<kind> flips territory on completion)\n\
                   @clock/advance <id>[=<n>]  --  advance (a completed clock fires its \
                   consequence: a canon event + any territory flip)\n\
                   @clock/del <id>"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_clock),
    )?;
    reg.command(
        CommandSpec {
            name: "+clocks".into(),
            shape: ArgShape::None,
            help: "+clocks  --  The faction pressure you know of: progress clocks (their filled \
                   segments) for factions whose affairs are visible to you."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(cmd_player_clocks),
    )?;
    Ok(())
}

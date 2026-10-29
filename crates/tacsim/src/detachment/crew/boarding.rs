//! A hired person uses the passenger service's existing physical boarding path.
use super::{CrewHandles, runtime::check_contract};
use mush_core::{Ctx, Flags, ObjId, ObjKind};

pub(super) fn board(
    ctx: &mut Ctx<'_>,
    npc: ObjId,
    handles: &CrewHandles,
) -> Result<String, String> {
    // Establish the caller's own agreement before consulting whereabouts.
    let contract = ctx
        .action_state(&handles.contracts, &npc)
        .map_err(|_| "Crew or contract unavailable.")?
        .filter(|contract| contract.employer == ctx.actor)
        .ok_or("Crew or contract unavailable.")?;
    check_contract(npc, Some(contract))?;
    if ctx
        .w
        .module_state::<crate::TacsimStore>()
        .is_none_or(|store| !store.enabled())
        || ctx
            .w
            .get(ctx.actor)
            .is_none_or(|o| o.kind != ObjKind::Player || o.flags.contains(Flags::GOING))
        || ctx
            .w
            .get(npc)
            .is_none_or(|o| o.kind != ObjKind::Npc || o.flags.contains(Flags::GOING))
    {
        return Err("Crew or contract unavailable.".into());
    }
    if ctx.authoritative_pending(ctx.actor) || ctx.authoritative_commit_pending() {
        return Err("An authoritative request is already pending.".into());
    }
    let hull = crate::port::coop::helm_of(ctx.w, ctx.actor)
        .ok_or("You need a current ship to arrange crew boarding.")?
        .hull;
    let crate::port::ship::Berth::Docked { def, .. } = crate::port::ship::berth_of(ctx.w, hull)
    else {
        return Err("Arrange crew boarding at a port with your ship alongside.".into());
    };
    let port = def.venue.as_str().to_owned();
    if crate::port::voyage::voyage_blocks(ctx.w, hull)
        || !(crate::port::passenger::at_station(ctx.w, ctx.actor, &port)
            || crate::port::voyage::aboard(ctx.w, hull, ctx.actor))
    {
        return Err(
            "Arrange crew boarding from this port or aboard your ship, before filing clearance."
                .into(),
        );
    }
    if crate::detachment::planning::projection::physically_aboard(ctx.w, npc, hull) {
        return Ok("That crew member is already aboard your ship.".into());
    }
    if !crate::port::passenger::at_station(ctx.w, npc, &port) {
        return Err("That crew member is not at this departure port.".into());
    }
    crate::port::passenger::board_person(ctx, npc, hull, &port)?;
    mush_npc::notify_role_event(
        ctx,
        npc,
        "Your employer has arranged boarding. You have boarded their ship under your crew agreement; settle into the crew compartment.",
    );
    Ok(format!(
        "{} has come aboard {} under your crew contract.",
        crate::port::coop::name_of(ctx.w, npc),
        crate::port::coop::name_of(ctx.w, hull)
    ))
}

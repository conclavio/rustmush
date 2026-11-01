use super::{
    facts,
    model::*,
    runtime::{self, SortieHandles},
};
use mush_core::{Ctx, ModError, Registrar, SceneId, TickCadence};
use std::{collections::BTreeSet, sync::Arc, time::Duration};

#[derive(Clone, Default)]
struct TickState {
    after: Option<mush_core::ObjId>,
}
pub(super) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.state(TickState::default());
    reg.tick(
        "tacsim.detachment_flights",
        TickCadence::Coarse(Duration::from_secs(5)),
        Arc::new(tick),
    )
}
fn tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    let Some(h) = ctx
        .w
        .module_state::<SortieHandles>()
        .filter(|h| h.enabled)
        .cloned()
    else {
        return;
    };
    if ctx
        .w
        .module_state::<crate::TacsimStore>()
        .is_none_or(|s| !s.enabled())
        || ctx.authoritative_commit_pending()
        || ctx.w.world_timeline().is_frozen()
    {
        return;
    }
    let after = ctx.w.module_state::<TickState>().and_then(|s| s.after);
    let Ok(Some(index)) = ctx.action_state(&h.index, &()) else {
        return;
    };
    if index.validate().is_err() {
        return;
    }
    let mut units = index
        .units
        .iter()
        .copied()
        .filter(|unit| after.is_none_or(|last| *unit > last))
        .take(33)
        .collect::<Vec<_>>();
    let more = units.len() > 32;
    units.truncate(32);
    let next = if more { units.last().copied() } else { None };
    let heads = units
        .iter()
        .filter_map(|unit| {
            ctx.action_state(&h.heads, unit)
                .ok()
                .flatten()
                .filter(|head| head.flight.unit == *unit && head.validate().is_ok())
                .cloned()
        })
        .collect::<Vec<_>>();
    if let Some(state) = ctx.w.module_state_mut::<TickState>() {
        state.after = next
    }
    let mut acted = BTreeSet::new();
    let mut operations = BTreeSet::new();
    for head in heads {
        let Ok(Some(flight)) = ctx
            .action_state(&h.flights, &head.flight)
            .map(|f| f.cloned())
        else {
            continue;
        };
        if flight.key != head.flight
            || flight.validate().is_err()
            || !flight.phase.open()
            || !known_clock(ctx.w, &flight.requested)
            || !known_clock(ctx.w, &flight.due)
            || matches!(flight.phase, Phase::Departing | Phase::Returning)
                && ctx.w.world_ic_now().at < flight.due.at
            || ctx.authoritative_pending(flight.continuation_actor())
            || acted.contains(&flight.continuation_actor())
        {
            continue;
        }
        let Ok(Some(op)) = ctx
            .action_state(&h.operations, &flight.operation)
            .map(|o| o.cloned())
        else {
            continue;
        };
        if op.id != flight.operation
            || op.validate().is_err()
            || super::prepare::membership(&op, &flight).is_err()
        {
            continue;
        }
        let contract = ctx
            .action_state(&h.contracts, &flight.operator)
            .ok()
            .flatten()
            .cloned();
        // The same bounded flight index discovers active operations. A refresh
        // never moves a unit and retains no second roster/location registry.
        if flight.phase == Phase::Field && operations.insert(op.id) {
            if super::field_orders::needs_poll(ctx.w, &op)
                && let Ok(request) = super::field_orders::request(
                    ctx,
                    &h,
                    &op,
                    op.id.organizer,
                    super::field_orders::Act::Poll,
                )
                && runtime::enqueue(
                    ctx,
                    op.id.organizer,
                    runtime::Command::Message(Box::new(request)),
                )
                .is_ok()
            {
                acted.insert(op.id.organizer);
                if acted.len() >= 8 {
                    break;
                }
                continue;
            }
            if let Ok(facts) = super::field::read_owner_facts(ctx, &h, &op)
                && let Ok((roster, _)) = super::field::reconcile(ctx.w, &mut &*ctx, &h, &op, &facts)
                && op
                    .command
                    .as_deref()
                    .is_none_or(|old| !old.same_succession(&roster.checkpoint()))
                && runtime::enqueue(
                    ctx,
                    op.id.organizer,
                    runtime::Command::Refresh {
                        operation: op.id,
                        facts,
                    },
                )
                .is_ok()
            {
                acted.insert(op.id.organizer);
                if acted.len() >= 8 {
                    break;
                }
                continue;
            }
        }
        let already = flight.phase == Phase::Departing && facts::on_site(ctx.w, &flight)
            || matches!(flight.phase, Phase::Field | Phase::Returning)
                && facts::in_bay(ctx.w, &flight);
        if !already {
            // An unlaunched departure waits once the operation stops standing, ended or
            // deactivated: deactivation moves nothing, and the organizer's `+sortie/cancel`
            // closes it (DT-2's residual edge).
            if flight.phase == Phase::Field
                || flight.phase == Phase::Departing
                    && !super::prepare::standing(&mut &*ctx, &h, &op).unwrap_or(false)
                || !facts::pending_can_move(ctx.w, &op.roster, &flight, contract)
            {
                continue;
            }
            let moved = match flight.phase {
                // Doc 32 §17: beside the ramp when another unit already stands on its cell.
                Phase::Departing => {
                    let pose = crate::field::carrier::launch_pose(
                        ctx,
                        &flight.site,
                        flight.domain,
                        &flight.ramp,
                        flight.key.unit,
                    );
                    mush_sim::place(ctx, flight.key.unit, flight.domain, pose)
                }
                Phase::Returning => mush_sim::place_interior(
                    ctx,
                    flight.key.unit,
                    flight.bay_room,
                    Some(flight.bay_slot.clone()),
                ),
                _ => continue,
            };
            if let Err(error) = moved {
                tracing::warn!(unit=flight.key.unit.0,%error,"detachment physical step refused");
                continue;
            }
        }
        if runtime::finish_submit(ctx, &flight).is_ok() {
            acted.insert(flight.continuation_actor());
        }
        if acted.len() >= 8 {
            break;
        }
    }
}

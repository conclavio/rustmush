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
    // M34: evict settled/expired `Pending` intents and emit post-commit lapse notices.
    runtime::prune_settled(ctx, &h);
    runtime::flush_committed_lapses(ctx, &h);
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
    if units.is_empty() && after.is_some() {
        units = index.units.iter().copied().take(33).collect();
    }
    let more = units.len() > 32;
    units.truncate(32);
    let last_unit = units.last().copied();
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
    let mut acted = BTreeSet::new();
    let mut operations = BTreeSet::new();
    let mut last_visited: Option<mush_core::ObjId> = None;
    let mut hit_cap = false;
    for head in heads {
        last_visited = Some(head.flight.unit);
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
            // Doc 40 cut FR.3 (Q-40-20): a launch the field sent back aboard is still on the
            // surface; the field puts it down again, and its bay is no return.
            || flight.phase == Phase::Field
                && crate::field::carrier::relaunching_unit(ctx.w, flight.key.unit)
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
            if super::field_orders::needs_poll(ctx.w, &mut &*ctx, &h, &op)
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
                    hit_cap = true;
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
                    hit_cap = true;
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
            // Doc 40 (the ramp exit's boarding window): a return whose carrier is lost places
            // nothing; its finish puts the flight back in the field (`prepare`'s finish).
            let lost = flight.phase == Phase::Returning
                && crate::field::boarding::carrier_lost(ctx.w, &flight.site, flight.carrier);
            let moved = match flight.phase {
                Phase::Returning if lost => Ok(()),
                Phase::Departing => {
                    mush_sim::place(ctx, flight.key.unit, flight.domain, flight.ramp.clone())
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
            hit_cap = true;
            break;
        }
    }
    // M35: persist cursor at the last unit actually visited when the per-tick actor cap broke
    // early, wrapping to `None` only after traversing all remaining units in the index.
    let next = next_cursor(hit_cap, more, last_visited, last_unit);
    if let Some(state) = ctx.w.module_state_mut::<TickState>() {
        state.after = next;
    }
}

/// Compute the next scan cursor for [`tick`] so breaking on the 8-actor cap resumes right after
/// `last_visited` instead of restarting at index 0 and starving higher-`ObjId` flights (M35).
fn next_cursor(
    hit_cap: bool,
    more: bool,
    last_visited: Option<mush_core::ObjId>,
    last_unit: Option<mush_core::ObjId>,
) -> Option<mush_core::ObjId> {
    if hit_cap {
        if !more && last_visited == last_unit {
            None
        } else {
            last_visited
        }
    } else if more {
        last_unit
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mush_core::ObjId;

    #[test]
    fn m35_cursor_resumes_after_cap_break_without_starving_higher_flights() {
        // <= 32 flights total (`more == false`), broke at actor cap on unit #8 out of #16:
        // cursor must stay at #8 so next tick visits #9..#16 instead of restarting at #1.
        assert_eq!(
            next_cursor(true, false, Some(ObjId(8)), Some(ObjId(16))),
            Some(ObjId(8))
        );
        // When the 8th acted unit happens to be the very last unit with no more remaining,
        // wrap immediately to `None`.
        assert_eq!(
            next_cursor(true, false, Some(ObjId(8)), Some(ObjId(8))),
            None
        );
        // > 32 flights (`more == true`), broke at #8 -> resume at #8 (not #32!).
        assert_eq!(
            next_cursor(true, true, Some(ObjId(8)), Some(ObjId(32))),
            Some(ObjId(8))
        );
        // Finished whole slice without hitting cap: advance past #32 if `more`, else wrap.
        assert_eq!(
            next_cursor(false, true, Some(ObjId(32)), Some(ObjId(32))),
            Some(ObjId(32))
        );
        assert_eq!(
            next_cursor(false, false, Some(ObjId(16)), Some(ObjId(16))),
            None
        );
    }
}

//! Decisions use delivered orders and the operator's own readings. All effects
//! still enter the existing source actions as the actual NPC.
use super::{FieldObjective, Phase, SortieHandles, field_orders, runtime};
use crate::port_tick::{Fire, Step};
use crate::surface::{
    act, chart,
    excursion::{ActKind, ActResult},
    geology::{OreFamily, Word},
};
use mush_core::{Ctx, ObjId, ObjKind};
use mush_tactics::detachment::{Controller, DetachmentRoster, Directive, OrderBook, Response};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default)]
pub(crate) struct Walk {
    after: Option<ObjId>,
    /// Scheduling only: a congested reply never becomes an execution permit.
    replies: BTreeMap<ObjId, (u64, i64)>,
}

pub(crate) fn tick(ctx: &mut Ctx<'_>, fire: &mut Fire, walk: &mut Walk, budget: u32) -> u32 {
    let Some(h) = ctx
        .w
        .module_state::<SortieHandles>()
        .filter(|h| h.enabled)
        .cloned()
    else {
        return 0;
    };
    let Ok(Some(index)) = ctx.action_state(&h.index, &()) else {
        return 0;
    };
    if index.validate().is_err() {
        return 0;
    }
    walk.replies.retain(|unit, _| index.units.contains(unit));
    let mut units = index
        .units
        .iter()
        .copied()
        .filter(|unit| walk.after.is_none_or(|last| *unit > last))
        .take(33)
        .collect::<Vec<_>>();
    let more = units.len() > 32;
    units.truncate(32);
    let mut worked = 0;
    for unit in units {
        if worked >= budget {
            return worked;
        }
        walk.after = Some(unit);
        let step = visit(ctx, fire, walk, &h, unit);
        fire.settle(unit, step);
        if step == Step::Submitted {
            worked += 1;
        }
    }
    if !more {
        walk.after = None;
    }
    worked
}

fn visit(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    walk: &mut Walk,
    h: &SortieHandles,
    unit: ObjId,
) -> Step {
    let Ok((_, flight)) = super::prepare::current(&mut &*ctx, h, unit) else {
        return Step::Idle;
    };
    if !matches!(flight.phase, Phase::Field | Phase::Returned) {
        return Step::Idle;
    }
    let actor = flight.operator;
    if !super::facts::live(ctx.w, actor, ObjKind::Npc) {
        return Step::Idle;
    }
    let Ok(op) = super::prepare::operation(&mut &*ctx, h, flight.operation) else {
        return Step::Idle;
    };
    if op.ended.is_some() {
        return Step::Idle;
    }
    let Some(saved) = op.command.as_deref() else {
        return Step::Idle;
    };
    let Ok(roster) = DetachmentRoster::restore(saved) else {
        return Step::Idle;
    };
    let Some(member) = roster.members().iter().find(|m| {
        m.unit == unit
            && matches!(m.controller, Controller::Npc { character } if character == actor)
    }) else {
        return Step::Idle;
    };
    let slot = member.slot;
    let Some(orders) = op.orders.as_deref() else {
        return Step::Idle;
    };
    let Ok(book) = OrderBook::restore(&orders.book, &roster) else {
        return Step::Idle;
    };
    let Some(delivered) = book.delivered(slot).cloned() else {
        return Step::Idle;
    };
    let now = ctx.w.world_ic_now().at;
    let sequence = delivered.order.sequence;
    let valid = delivered.completed_at_ms.is_none()
        && delivered.order.expires_at_ms.is_none_or(|end| {
            u64::try_from(now)
                .ok()
                .and_then(|n| n.checked_mul(1000))
                .is_some_and(|n| n < end)
        })
        && orders
            .flights
            .get(&slot)
            .and_then(|f| f.standing.as_ref())
            .is_some_and(|f| {
                f.sequence == sequence && f.flight == flight.key && f.site == flight.site
            });
    // An expired/replaced task may stop its old run, but admits no new work.
    let record = ctx
        .w
        .module_state::<crate::port::PortHandles>()
        .and_then(|p| {
            ctx.action_state(
                &p.families.excursions,
                &crate::surface::excursion::RigKey(unit),
            )
            .ok()
            .flatten()
        })
        .cloned();
    let same = record
        .as_ref()
        .and_then(|r| r.field_work.as_deref())
        .filter(|w| w.sequence == sequence && w.flight == flight.key);
    let failed = record.as_ref().is_some_and(|r| {
        same.is_some_and(|w| {
            r.last.as_ref().is_some_and(|last| {
                last.ordinal == w.ordinal
                    && (last.result != ActResult::Done
                        || (last.kind == ActKind::Mine && !w.mined && r.run.is_none()))
            })
        })
    });
    let negative = delivered
        .acknowledgement
        .is_some_and(|a| a.response == Response::Negative);
    let objective = match &delivered.order.directive {
        Directive::Objective(p) => Some(&p.objective),
        Directive::Cancel => None,
    };
    // Doc 32 cut CF.6: a unit standing in a field flies the doctrine under its order; the
    // field's executor offers it to the kernel and reads the answer off the NPC's own frame.
    if let Some(site) = crate::field::field_of_unit(ctx.w, unit) {
        if !valid || negative {
            return Step::Idle;
        }
        let step = crate::field::orders::step(ctx, &site, (unit, actor), &roster, &delivered);
        return match step {
            crate::field::orders::Step::Idle => Step::Idle,
            crate::field::orders::Step::Waiting => Step::Held,
            crate::field::orders::Step::Reply(response) => answer(
                ctx,
                fire,
                walk,
                (h, &op),
                (unit, actor, slot, sequence),
                response,
            ),
        };
    }
    // §4a: a delivered ENGAGE naming a presence row on open ground is its leader's Hostile
    // mark on that flight's party, seen from this unit; the answer follows the mark.
    if valid
        && !negative
        && delivered.acknowledgement.is_none()
        && let Some(engage @ FieldObjective::Engage { at: Some(_), .. }) = objective
        && record
            .as_ref()
            .is_none_or(|r| r.act.is_none() && r.queued.is_none() && r.run.is_none())
    {
        if walk
            .replies
            .get(&unit)
            .is_some_and(|(seq, until)| *seq == sequence && *until > now)
        {
            return Step::Held;
        }
        let marked = crate::field::orders::surface_mark(
            ctx,
            &flight.site,
            unit,
            delivered.order.issuer,
            engage,
        );
        if let Err(error) = &marked {
            tracing::debug!(unit = unit.0, %error, "NPC surface ENGAGE refused");
        }
        let response = if matches!(marked, Ok(true)) {
            Response::Wilco
        } else {
            Response::Negative
        };
        return answer(
            ctx,
            fire,
            walk,
            (h, &op),
            (unit, actor, slot, sequence),
            response,
        );
    }
    // A mining run is a mining roster's (a combat contract never mines, doc 27b FC.1).
    let mines = op.roster.duty == crate::detachment::crew::CrewDuty::Mining;
    let supported = match objective {
        None
        | Some(FieldObjective::Hold | FieldObjective::Advance { .. } | FieldObjective::Withdraw) => {
            true
        }
        Some(FieldObjective::Mine { .. }) => mines,
        Some(_) => false,
    };
    let stop = !valid
        || negative
        || failed
        || !supported
        || matches!(objective, None | Some(FieldObjective::Hold))
        || record
            .as_ref()
            .is_some_and(|r| r.run.is_some() && same.is_none());
    if record
        .as_ref()
        .is_some_and(|r| r.act.is_some() || r.queued.is_some())
    {
        return Step::Held;
    }
    if record.as_ref().is_some_and(|r| r.run.is_some()) {
        if !stop
            || record
                .as_ref()
                .is_some_and(|r| r.run.as_ref().is_some_and(|run| run.stop))
        {
            return Step::Held;
        }
        return dispatch(ctx, fire, unit, actor, |ctx| {
            let mut rig =
                super::surface::rig(ctx).ok_or_else(|| "Physical crew unavailable.".to_owned())?;
            // Safety stop is legal even when the old task is expired/unsupported.
            if valid && supported && !negative && !failed {
                rig.field_order = Some(sequence);
            }
            act::mine_stop_cmd(ctx, &rig).map_err(|e| e.to_string())
        });
    }
    if !valid || negative {
        return Step::Idle;
    }
    let reply = if !supported || failed {
        Some(Response::Negative)
    } else if delivered.acknowledgement.is_none() {
        Some(Response::Wilco)
    } else {
        None
    };
    if let Some(response) = reply {
        if walk
            .replies
            .get(&unit)
            .is_none_or(|(seq, until)| *seq != sequence || *until <= now)
        {
            walk.replies
                .insert(unit, (sequence, now.saturating_add(60)));
            return dispatch(ctx, fire, unit, actor, |ctx| {
                let request = field_orders::request(
                    ctx,
                    h,
                    &op,
                    actor,
                    field_orders::Act::Respond {
                        slot,
                        sequence,
                        response,
                    },
                )?;
                runtime::submit(ctx, runtime::Command::Message(Box::new(request)))
            });
        }
        if response == Response::Negative {
            return Step::Held;
        }
    }
    if matches!(objective, None | Some(FieldObjective::Hold)) {
        return Step::Idle;
    }
    dispatch(ctx, fire, unit, actor, |ctx| {
        let result = (|| -> Result<bool, String> {
            let mut rig =
                super::surface::rig(ctx).ok_or_else(|| "Physical crew unavailable.".to_owned())?;
            rig.field_order = Some(sequence);
            let stand = act::stand_of(ctx.w, actor, &rig).map_err(|r| r.line())?;
            // Ground building belongs to the existing warm pass, never the NPC walk.
            let mut peeks = crate::surface::Peeks::default();
            if matches!(
                crate::surface::peek_home(ctx.w, &stand.site, stand.ramp_cell, &mut peeks),
                crate::surface::HomePeek::Cold
            ) {
                crate::surface::want_site_ground(ctx.w, &stand.site);
                crate::surface::want_home(ctx.w, &stand.site, stand.ramp_cell);
                return Ok(false);
            }
            match objective {
                Some(FieldObjective::Advance { to }) => {
                    act::drive_cmd(ctx, &rig, &format!("{},{}", to.0, to.1), true)
                }
                Some(FieldObjective::Withdraw) => {
                    act::act_cmd(ctx, &rig, ActKind::Recover, false, true, None)
                }
                Some(FieldObjective::Mine { family }) => mine(
                    ctx,
                    &rig,
                    &stand.site,
                    stand.cell,
                    *family,
                    record.as_ref(),
                    sequence,
                ),
                _ => return Ok(false),
            }
            .map_err(|e| e.to_string())?;
            Ok(true)
        })();
        match result {
            Ok(submitted) => Ok(submitted),
            Err(error) => {
                tracing::debug!(unit=unit.0, %error, "NPC task cannot proceed");
                let request = field_orders::request(
                    ctx,
                    h,
                    &op,
                    actor,
                    field_orders::Act::Respond {
                        slot,
                        sequence,
                        response: Response::Negative,
                    },
                )?;
                runtime::submit(ctx, runtime::Command::Message(Box::new(request)))?;
                Ok(true)
            }
        }
    })
}

/// **One reply on the transport**, once per order sequence while it is in flight: the NPC's
/// WILCO or NEGATIVE, submitted as the NPC.
fn answer(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    walk: &mut Walk,
    (h, op): (&SortieHandles, &super::OperationRecord),
    (unit, actor, slot, sequence): (ObjId, ObjId, mush_tactics::detachment::Slot, u64),
    response: Response,
) -> Step {
    let now = ctx.w.world_ic_now().at;
    if walk
        .replies
        .get(&unit)
        .is_some_and(|(seq, until)| *seq == sequence && *until > now)
    {
        return Step::Held;
    }
    walk.replies
        .insert(unit, (sequence, now.saturating_add(60)));
    dispatch(ctx, fire, unit, actor, |ctx| {
        let request = field_orders::request(
            ctx,
            h,
            op,
            actor,
            field_orders::Act::Respond {
                slot,
                sequence,
                response,
            },
        )?;
        runtime::submit(ctx, runtime::Command::Message(Box::new(request)))
    })
}

trait Submitted {
    fn submitted(self) -> bool;
}
impl Submitted for () {
    fn submitted(self) -> bool {
        true
    }
}
impl Submitted for bool {
    fn submitted(self) -> bool {
        self
    }
}
fn dispatch<T: Submitted>(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    unit: ObjId,
    actor: ObjId,
    call: impl FnOnce(&mut Ctx<'_>) -> Result<T, String>,
) -> Step {
    match fire.as_principal(ctx, unit, actor, call) {
        Ok(Ok(result)) => {
            if result.submitted() {
                Step::Submitted
            } else {
                Step::Held
            }
        }
        Ok(Err(error)) => {
            tracing::debug!(unit=unit.0, %error, "NPC field decision refused");
            Step::Refused
        }
        Err(_) => Step::Deferred,
    }
}

fn mine(
    ctx: &mut Ctx<'_>,
    rig: &act::RigAt,
    site: &mush_core::CanonicalId,
    cell: (i64, i64),
    family: Option<OreFamily>,
    record: Option<&crate::surface::excursion::RigRecord>,
    sequence: u64,
) -> Result<(), mush_core::CmdError> {
    let linked = record.and_then(|r| r.field_work.as_deref()).filter(|w| {
        w.sequence == sequence && rig.crew.as_ref().is_some_and(|c| c.flight == w.flight)
    });
    if linked.is_some_and(|w| w.mined) {
        return act::act_cmd(ctx, rig, ActKind::Recover, false, true, None);
    }
    let row = ctx
        .w
        .module_state::<crate::port::PortHandles>()
        .and_then(|h| {
            ctx.action_state(
                &h.families.charts,
                &chart::ChartKey {
                    principal: ctx.actor,
                    site: site.clone(),
                },
            )
            .ok()
            .flatten()
        })
        .cloned();
    if row.as_ref().is_none_or(|r| !r.has_coverage()) {
        return act::act_cmd(ctx, rig, ActKind::Scan, false, true, None);
    }
    if row.as_ref().is_none_or(|r| r.sounded_pockets() == 0) {
        return act::act_cmd(ctx, rig, ActKind::Sound, false, true, None);
    }
    let context = chart::site_context(ctx.w, site)
        .map_err(|_| mush_core::CmdError::msg("Own readings unavailable."))?;
    let mut cache = BTreeMap::new();
    let mut candidates = Vec::new();
    for y in 0..64 {
        for x in 0..64 {
            if let Some(classes) =
                crate::surface::plan::read_classes(&context, row.as_ref(), (x, y), &mut cache)
            {
                let wanted = family.or_else(|| {
                    OreFamily::ALL
                        .into_iter()
                        .find(|f| crate::surface::geology::by_family(classes, *f) != Word::Barren)
                });
                if let Some(f) = wanted
                    .filter(|f| crate::surface::geology::by_family(classes, *f) != Word::Barren)
                {
                    let target = (x * 4 + 2, y * 4 + 2);
                    candidates.push((
                        (target.0 - cell.0).abs() + (target.1 - cell.1).abs(),
                        x,
                        y,
                        f,
                    ));
                }
            }
        }
    }
    candidates.sort_by_key(|c| (c.0, c.1, c.2));
    for (_, x, y, f) in candidates.into_iter().take(8) {
        if crate::surface::address::pocket_of_cell(cell) == Some((x, y)) {
            return act::mine_cmd(
                ctx,
                rig,
                &crate::surface::mine::MineChoice {
                    family: Some(f),
                    word: Some(f.id().into()),
                    cycles: Some(1),
                },
                "1",
                true,
            );
        }
        let Some(address) = crate::surface::address::pocket_address(x, y) else {
            continue;
        };
        if act::drive_cmd(ctx, rig, &address, true).is_ok() {
            return Ok(());
        }
    }
    Err(mush_core::CmdError::msg(
        "No reachable ore in this operator's own sounding.",
    ))
}

//! **The compiled NPC commander's host half** (doc 44 unit MW.14a; Q-44-44): the issuer.
//!
//! [`crate::detachment::commander`] is the rule. This is the seat it sits in: once a roster's
//! field command names an NPC its leader (the plan's `npc_commander`, or its successor), the
//! NPC walk ([`super::npc`]) calls [`step`] on the leader's own unit before it executes that
//! unit's own order. The step builds the commander's view, takes the rule's posture, and gives
//! it as a roster order to the first hull that does not yet have it, **through the delivery a
//! player leader's orders take and nothing beside it**:
//!
//! - **the issuer's authority.** The order is submitted as the NPC, the roster's current leader,
//!   fenced by the roster's revision and command term exactly as a typed `ORDER DT:…` is
//!   ([`super::field_orders::Act::Issue`]); Org's mission rights are read for the NPC over every
//!   unit, and a hull the NPC holds no command right over is passed over;
//! - **the comms path.** The admitted order is a `mush_propagation` envelope to the recipient.
//!   Interference at the recipient's place holds it (`@prop/jam`), and an order that is not
//!   delivered is never executed: the recipient keeps the order it had;
//! - **the recipient's own contacts.** The recipient's executor ([`crate::field::orders::step`])
//!   offers the delivered order to the kernel, which resolves it in the recipient's own
//!   observation and refuses it if its issuer's hull is a wreck (`NoCommander`).
//!
//! **One order a visit.** A principal acts once a fire ([`crate::port_tick::Fire`]), so a
//! posture reaches a roster of `n` hulls over `n` fires, slot by slot in the roster's order, the
//! commander's own slot among them. The order book is the commander's only memory: a hull whose
//! pending or delivered order already is the posture is not ordered again, so nothing is kept in
//! RAM and a restart re-derives the same next order.
//!
//! **It yields without being told to.** A returning player who is eligible takes the lead by the
//! roster's own rule (doc 27 Q-D4, FO-15; the return hysteresis); the NPC is then not the leader,
//! [`step`] finds that and gives nothing, and what it had in flight under its own term is
//! discarded by the poll as any deposed leader's is.
//!
//! **Only inside a running field** (doc 44 §4 D). On open ground the surface executor runs a
//! hired NPC's delivered orders as surface acts, and nothing here orders one.

use mush_core::{Ctx, ObjId};
use mush_tactics::detachment::{
    DetachmentRoster, Directive, LeaderKind, Member, OrderBook, OrderTarget,
};

use super::field_orders::{self, Act, FieldObjective, FieldPayload};
use super::{Flight, OperationRecord, SortieHandles, runtime};
use crate::detachment::commander::{Posture, Strength, View, WORK_POINT_CELLS, decide};
use crate::port_tick::{Fire, Step};

/// **The posture as the roster order a player leader would type** (`ADVANCE x,y`, `HOLD`,
/// `WITHDRAW`), holding no fire.
fn directive(posture: Posture) -> Directive<FieldPayload> {
    let objective = match posture {
        Posture::Advance { to } => FieldObjective::Advance { to },
        Posture::Hold => FieldObjective::Hold,
        Posture::Withdraw => FieldObjective::Withdraw,
    };
    Directive::Objective(FieldPayload {
        objective,
        hold_fire: false,
    })
}

/// **The detachment's work point** (the rule's [`View::work`]): the site cell
/// [`WORK_POINT_CELLS`] out from the flight's ramp, straight away from the pad its carrier stands
/// on, kept inside the site. A ramp on the pad's own centre gives the ramp's cell.
///
/// The pad, not the carrier's placement: a settled hull's own placement is the domain's passive
/// pose (`crate::field::carrier::pad_poses`), and the site's pad is where the surface draws it.
fn work_point(ctx: &Ctx<'_>, flight: &Flight) -> (u16, u16) {
    let cell_cm = crate::surface::ground::SITE_CELL_CM;
    let (rx, ry) = (flight.ramp.x_cm, flight.ramp.y_cm);
    let (px, py) = crate::surface::excursion::site_pad_cm(ctx.w, &flight.site);
    let away = Some((rx - px, ry - py)).filter(|(dx, dy)| *dx != 0 || *dy != 0);
    let (x_cm, y_cm) = away.map_or((rx, ry), |(dx, dy)| {
        let (dx, dy) = (i128::from(dx), i128::from(dy));
        let len = i128::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(1);
        let out = i128::from(WORK_POINT_CELLS) * i128::from(cell_cm);
        let step = |d: i128| i64::try_from(d * out / len.max(1)).unwrap_or(0);
        (rx.saturating_add(step(dx)), ry.saturating_add(step(dy)))
    });
    let side = i64::from(crate::surface::site_record::site_side_or_default(
        ctx.w,
        &flight.site,
    ));
    let cell =
        |cm: i64| u16::try_from(cm.div_euclid(i64::from(cell_cm)).clamp(0, side - 1)).unwrap_or(0);
    (cell(x_cm), cell(y_cm))
}

/// **The commander's view** (the rule's input): its own frame's Hostile contacts, the roster's
/// hulls and those lost in this field, and the work point. `None` until the commander's own
/// frame has arrived: it does not decide blind.
fn view(
    ctx: &Ctx<'_>,
    roster: &DetachmentRoster,
    flight: &Flight,
    (unit, actor): (ObjId, ObjId),
) -> Option<View> {
    let sight = crate::field::orders::sight(ctx.w, unit, actor)?;
    let lost = roster
        .members()
        .iter()
        .filter(|member| {
            crate::field::orders::lost(
                ctx.w,
                member.unit,
                member.controller.character().unwrap_or(member.unit),
            )
        })
        .count();
    Some(View {
        sight,
        strength: Strength {
            hulls: u16::try_from(roster.members().len()).unwrap_or(u16::MAX),
            lost: u16::try_from(lost).unwrap_or(u16::MAX),
        },
        work: work_point(ctx, flight),
    })
}

/// **Whether `member` still owes `wanted`**: it is a hull the commander can reach (available in
/// the last reconciliation, standing in the commander's field), and neither the order in flight
/// to it under this leader's term nor the order delivered to it already is `wanted`.
fn owes(
    ctx: &Ctx<'_>,
    site: &mush_core::CanonicalId,
    roster: &DetachmentRoster,
    book: &OrderBook<FieldPayload>,
    issuer: ObjId,
    member: &Member,
    wanted: &Directive<FieldPayload>,
) -> bool {
    if !roster.member_available(member.slot)
        || crate::field::field_of_unit(ctx.w, member.unit).as_ref() != Some(site)
    {
        return false;
    }
    let in_flight = book.pending(member.slot).is_some_and(|pending| {
        pending.directive == *wanted
            && pending.issuer == issuer
            && pending.command_term == roster.term()
    });
    let standing = book
        .delivered(member.slot)
        .is_some_and(|delivered| delivered.order.directive == *wanted);
    // An order in flight that is not the posture is superseded by the next issue; one that is
    // the posture is waited for, whatever stands meanwhile.
    !in_flight && (book.pending(member.slot).is_some() || !standing)
}

/// **The commander's turn on its own unit** (this module's head): `None` when `actor` is not
/// the roster's NPC leader in a running field, when its frame has not arrived, or when every
/// hull it can reach already has the posture; else the step its one order took.
pub(super) fn step(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    h: &SortieHandles,
    op: &OperationRecord,
    roster: &DetachmentRoster,
    flight: &Flight,
    (unit, actor): (ObjId, ObjId),
) -> Option<Step> {
    let leader = roster.leader()?;
    if leader.kind != LeaderKind::Npc || leader.character != actor || op.ended.is_some() {
        return None;
    }
    let site = crate::field::field_of_unit(ctx.w, unit)?;
    let wanted = directive(decide(&view(ctx, roster, flight, (unit, actor))?));
    let book = match op.orders.as_deref() {
        Some(orders) => OrderBook::restore(&orders.book, roster).ok()?,
        None => OrderBook::new(roster.id()),
    };
    let owing: Vec<&Member> = roster
        .members()
        .iter()
        .filter(|member| owes(ctx, &site, roster, &book, actor, member, &wanted))
        .collect();
    let first = owing.first()?.slot;
    let (revision, term) = (roster.revision(), roster.term());
    Some(super::npc::dispatch(ctx, fire, unit, actor, |ctx| {
        let mut request = field_orders::request(
            ctx,
            h,
            op,
            actor,
            Act::Issue {
                revision,
                term,
                target: OrderTarget::Slot(first),
                directive: wanted.clone(),
                expires_at_ms: None,
            },
        )?;
        // Org's answer for this issuer covers every unit of the roster: the order goes to the
        // first owing hull the NPC holds a command right over, and to none when it holds none.
        let slot = owing
            .iter()
            .find(|member| {
                request
                    .rights
                    .iter()
                    .any(|right| right.unit == member.unit && right.allowed)
            })
            .map(|member| member.slot)
            .ok_or_else(|| "The commander holds no command right over an owing unit.".to_owned())?;
        if let Act::Issue { target, .. } = &mut request.act {
            *target = OrderTarget::Slot(slot);
        }
        runtime::submit(ctx, runtime::Command::Message(Box::new(request)))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **Doc 44 unit MW.14a**: each posture is the roster order a player leader types, and none
    /// holds fire.
    #[test]
    fn mw14a_a_posture_is_a_roster_order_a_player_leader_can_give() {
        let objective = |posture| match directive(posture) {
            Directive::Objective(payload) => {
                assert!(!payload.hold_fire);
                payload.objective
            }
            Directive::Cancel => panic!("a posture is never a cancellation"),
        };
        assert_eq!(
            objective(Posture::Advance { to: (128, 118) }),
            FieldObjective::Advance { to: (128, 118) }
        );
        assert_eq!(objective(Posture::Hold), FieldObjective::Hold);
        assert_eq!(objective(Posture::Withdraw), FieldObjective::Withdraw);
    }
}

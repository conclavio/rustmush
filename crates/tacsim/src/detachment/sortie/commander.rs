//! **The compiled NPC commander's host half** (doc 44 units MW.14a and MW.14c; Q-44-44,
//! Q-44-49): the issuer.
//!
//! [`crate::detachment::commander`] is the rule. This is the seat it sits in: once a roster's
//! field command names an NPC its leader (the plan's `npc_commander`, or its successor), the
//! NPC walk ([`super::npc`]) calls [`step`] on the leader's own unit before it executes that
//! unit's own order. The step builds the commander's view, takes the rule's decision, and gives
//! it as roster orders, the posture first and then the contact to concentrate on, **through
//! the delivery a player leader's orders take and nothing beside it**:
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
//! commander's own slot among them. A `CONCENTRATE` that every hull it can reach still owes
//! goes to all of them in one order (the tolerant target, which reaches whoever can take it);
//! one that only some owe goes slot by slot, so a hull already on the contact is not ordered
//! again.
//!
//! **The roster's order history is the commander's only memory** (doc 44 unit MW.14c): the
//! order book, and the record of what its NPC leaders ordered that the same transaction keeps
//! beside it. A hull whose pending or delivered order already is the posture is not ordered
//! again; the posture it last ordered and the contact it last named, and since when, are read
//! from that record ([`remembered`]); so a restart re-derives the same view and the same next
//! order. A hull under a delivered `CONCENTRATE` stands on the posture it goes back to. The one
//! thing kept in RAM is scheduling ([`Sent`]): an order
//! just submitted to a slot is not submitted again for [`RESEND_AFTER_S`] of wall time, and an
//! attempt that submitted nothing is not repeated for [`RETRY_AFTER_S`]. So an order refused at
//! its final take, or answered NEGATIVE, is retried at that pace and not every fire, and the
//! commander's own hull goes on flying its order in the fires between.
//!
//! **It yields without being told to.** A returning player who is eligible takes the lead by the
//! roster's own rule (doc 27 Q-D4, FO-15; the return hysteresis); the NPC is then not the leader,
//! [`step`] finds that and gives nothing, and what it had in flight under its own term is
//! discarded by the poll as any deposed leader's is.
//!
//! **Only inside a running field** (doc 44 §4 D). On open ground the surface executor runs a
//! hired NPC's delivered orders as surface acts, and nothing here orders one.

use std::collections::BTreeMap;

use mush_core::{Ctx, ObjId};
use mush_tactics::detachment::{
    DetachmentRoster, Directive, LeaderKind, Member, OrderBook, OrderTarget, Response, Slot,
};

use super::field_orders::{self, Act, FieldObjective, FieldOrders, FieldPayload};
use super::{Flight, OperationRecord, SortieHandles, runtime};
use crate::detachment::commander::{
    Held, Named, Posture, Strength, View, WORK_POINT_CELLS, decide,
};
use crate::port_tick::{Fire, Step};

/// **How long the commander waits before sending a slot the same order again**, wall seconds.
/// An order that was admitted is in the book and is not owed at all; this paces one that was
/// submitted and never landed, and one the recipient answered NEGATIVE. Wall time, not IC: at
/// the live rate of 24:1 an IC minute is under three fires, which would be no pacing at all.
pub const RESEND_AFTER_S: i64 = 60;

/// **How long the commander waits after an attempt that submitted nothing** (its owner reads
/// failed, the queue was busy), wall seconds: short, since nothing is in flight, and long enough
/// that its own hull flies its order in the fires between.
pub const RETRY_AFTER_S: i64 = 5;

/// **What a commander last sent each slot** (RAM, the NPC walk's): the commander's unit and the
/// slot → the order and the world second before which it is not sent again. Scheduling only: it
/// admits nothing, and a restart forgets it.
pub(super) type Sent = BTreeMap<(ObjId, u16), (Directive<FieldPayload>, i64)>;

/// What one turn of the issuer came to, for the resend guard.
enum Turn {
    /// The order was submitted to these slots.
    Sent(Vec<Slot>),
    /// The NPC holds no command right over any owing hull.
    NoRight,
}

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

/// **The posture a roster order is**, for one the commander could have given ([`directive`]'s
/// inverse): `None` for any other order, and for one that holds fire.
fn posture_of(order: &Directive<FieldPayload>) -> Option<Posture> {
    match order {
        Directive::Objective(FieldPayload {
            objective,
            hold_fire: false,
        }) => match objective {
            FieldObjective::Advance { to } => Some(Posture::Advance { to: *to }),
            FieldObjective::Hold => Some(Posture::Hold),
            FieldObjective::Withdraw => Some(Posture::Withdraw),
            _ => None,
        },
        _ => None,
    }
}

/// **The contact a `CONCENTRATE` names**, by its field and its row key in its leader's scope:
/// `None` for any other order.
fn named_by(order: &Directive<FieldPayload>) -> Option<(u32, u64)> {
    match order {
        Directive::Objective(FieldPayload {
            objective: FieldObjective::Concentrate { ordinal, id, .. },
            ..
        }) => Some((*ordinal, *id)),
        _ => None,
    }
}

/// **The detachment's default work point**: the site cell [`WORK_POINT_CELLS`] out from the
/// flight's ramp, straight away from the pad its carrier stands on, kept inside the site. A
/// ramp on the pad's own centre gives the ramp's cell.
///
/// The pad, not the carrier's placement: a settled hull's own placement is the domain's passive
/// pose (`crate::field::carrier::pad_poses`), and the site's pad is where the surface draws it.
fn default_work_point(ctx: &Ctx<'_>, flight: &Flight, side: i64) -> (u16, u16) {
    let cell_cm = crate::surface::ground::SITE_CELL_CM;
    let (rx, ry) = (flight.ramp.x_cm, flight.ramp.y_cm);
    let (px, py) = crate::surface::excursion::site_pad_cm(ctx.w, &flight.site);
    let away = Some((rx.saturating_sub(px), ry.saturating_sub(py)))
        .filter(|(dx, dy)| *dx != 0 || *dy != 0);
    let (x_cm, y_cm) = away.map_or((rx, ry), |(dx, dy)| {
        let (dx, dy) = (i128::from(dx), i128::from(dy));
        let len = i128::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(1);
        let out = i128::from(WORK_POINT_CELLS) * i128::from(cell_cm);
        let step = |d: i128| i64::try_from(d * out / len.max(1)).unwrap_or(0);
        (rx.saturating_add(step(dx)), ry.saturating_add(step(dy)))
    });
    // A site that reads no side at all still gives a cell: its first.
    let last = (side - 1).max(0);
    let cell =
        |cm: i64| u16::try_from(cm.div_euclid(i64::from(cell_cm)).clamp(0, last)).unwrap_or(0);
    (cell(x_cm), cell(y_cm))
}

/// **The work point a plan names, when it lies on a site `side` cells wide** (doc 44 unit
/// MW.14c). A plan is made before its site is known and is held only to the widest site, so a
/// point may lie off the site the roster flew to; it is then not the work point.
fn named_work_point(named: Option<(u16, u16)>, side: i64) -> Option<(u16, u16)> {
    named.filter(|(x, y)| i64::from(*x) < side && i64::from(*y) < side)
}

/// **The detachment's work point** (the rule's [`View::work`]; doc 44 unit MW.14c): the cell
/// the plan names when it lies on this site ([`named_work_point`]), else the default
/// ([`default_work_point`]).
fn work_point(ctx: &Ctx<'_>, op: &OperationRecord, flight: &Flight) -> (u16, u16) {
    let side = i64::from(crate::surface::site_record::site_side_or_default(
        ctx.w,
        &flight.site,
    ));
    named_work_point(op.roster.plan.work_point, side)
        .unwrap_or_else(|| default_work_point(ctx, flight, side))
}

/// Something the history records, and the IC millisecond its current spell began at.
type Since<T> = Option<(T, u64)>;

/// **What the roster's history records its NPC leader last ordered** (doc 44 unit MW.14c):
/// the posture and the contact to concentrate on, each with the IC millisecond its current
/// spell began at.
///
/// Read from the orders the history keeps of the roster's NPC leaders
/// ([`FieldOrders::given`], the last sixteen, each with what it said and when it was
/// admitted): the record the organiser reads in `+sortie/orders`. It is written in the
/// transaction that admits an order and nowhere else, and nothing here is kept in RAM, so a
/// restart reads the same. The order book alone could not say when a spell began: it holds
/// one order a slot, so an order of an earlier spell left on a hull the later orders never
/// reached would make the posture look old, and a Hostile passing a second later would flip
/// it.
///
/// - **The posture** is the latest posture any NPC leader of this roster ordered, the one
///   before `issuer` included: a successor inherits the dwell of the commander it succeeds.
///   Its spell began at the earliest order saying the same since the last order that said
///   another posture. A posture reaches a roster one hull a fire, and one re-sent to a hull
///   that answered NEGATIVE is the same spell.
/// - **The contact** is the one `issuer`'s own latest `CONCENTRATE` names, while no posture
///   but a `HOLD` was ordered after it: a row key is its leader's own scope's, so another
///   leader's names nothing to this one. Its spell began at the earliest order naming it
///   since the last that named another contact or ordered another posture.
fn remembered(orders: Option<&FieldOrders>, issuer: ObjId) -> (Since<Posture>, Since<u64>) {
    let given = orders.map_or(&[][..], |orders| orders.given.as_slice());
    let mut posture: Option<(Posture, u64)> = None;
    let mut contact: Option<((u32, u64), u64)> = None;
    for order in given {
        if let Some(said) = posture_of(&order.directive) {
            if posture.is_none_or(|(held, _)| held != said) {
                posture = Some((said, order.at_ms));
            }
            if said != Posture::Hold {
                contact = None;
            }
        } else if let Some(named) = named_by(&order.directive) {
            if order.issuer != issuer {
                contact = None;
            } else if contact.is_none_or(|(held, _)| held != named) {
                contact = Some((named, order.at_ms));
            }
        } else {
            // An order of another kind: no leader this rule speaks for gave it.
            posture = None;
            contact = None;
        }
    }
    (posture, contact.map(|((_, key), since)| (key, since)))
}

/// **Wall seconds since IC millisecond `since`**, at the world's rate now: the book stamps its
/// orders in IC time, and the commander's dwell is wall time
/// ([`crate::detachment::commander::DWELL_S`]).
fn wall_s_since(ctx: &Ctx<'_>, since_ic_ms: u64) -> u32 {
    let now_ic_ms = u64::try_from(ctx.w.world_ic_now().at)
        .unwrap_or(0)
        .saturating_mul(1_000);
    let rate = u64::from(ctx.w.world_timeline().current().rate).max(1);
    u32::try_from(now_ic_ms.saturating_sub(since_ic_ms) / 1_000 / rate).unwrap_or(u32::MAX)
}

/// **The commander's view** (the rule's input): its own frame's Hostile contacts, the roster's
/// hulls and those lost in this field, the work point, and what the book records it last
/// ordered. `None` until the commander's own frame has arrived: it does not decide blind.
fn view(
    ctx: &Ctx<'_>,
    (op, roster, flight): (&OperationRecord, &DetachmentRoster, &Flight),
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
    let (posture, contact) = remembered(op.orders.as_deref(), actor);
    Some(View {
        sight,
        strength: Strength {
            hulls: u16::try_from(roster.members().len()).unwrap_or(u16::MAX),
            lost: u16::try_from(lost).unwrap_or(u16::MAX),
        },
        work: work_point(ctx, op, flight),
        held: posture.map(|(posture, since)| Held {
            posture,
            for_s: wall_s_since(ctx, since),
        }),
        named: contact.map(|(key, since)| Named {
            key,
            for_s: wall_s_since(ctx, since),
        }),
    })
}

/// **Whether a roster's unit carries anything to fire**, by its own record. The commander
/// concentrates the hulls that do, and leaves a mining rig on its posture: an engagement would
/// send a hull with nothing to fire at the Hostile it was told to hold for.
fn can_fire(ctx: &Ctx<'_>, unit: ObjId) -> bool {
    ctx.w
        .module_state::<crate::TacsimStore>()
        .zip(mush_tactics::module::unit_of(ctx.w, unit))
        .is_some_and(|(store, cells)| {
            !crate::match_::weapon_summaries(store.definitions(), cells).is_empty()
        })
}

/// **What one hull owes of the commander's decision.**
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Owes {
    /// Nothing: it is out of reach, or it has what was decided, or that is on its way.
    Nothing,
    /// The posture.
    Posture,
    /// The contact to concentrate on (its posture stands).
    Concentrate,
}

/// **What `member` still owes of `wanted`**, the posture and the `CONCENTRATE` on top of it.
///
/// A hull the commander can reach (available in the last reconciliation, standing in the
/// commander's field) owes the posture when the posture is neither in flight to it under this
/// leader's term nor standing on it. A hull under a delivered `CONCENTRATE` stands on the
/// posture it goes back to. An order in flight that is neither the posture nor a `CONCENTRATE`
/// of this leader's is owed a replacement even when the posture stands meanwhile, and a
/// delivered posture the recipient answered NEGATIVE does not stand: a player leader would read
/// the NEGATIVE and order again, and so does the commander, at the resend guard's pace.
///
/// A hull whose posture stands owes the `CONCENTRATE` when it carries something to fire
/// ([`can_fire`]), the order is neither in flight to it nor delivered and unrefused, and
/// nothing else is in flight to it (the `CONCENTRATE` would supersede it).
fn owes(
    ctx: &Ctx<'_>,
    site: &mush_core::CanonicalId,
    roster: &DetachmentRoster,
    (book, orders): (&OrderBook<FieldPayload>, Option<&FieldOrders>),
    issuer: ObjId,
    member: &Member,
    (posture, contact): (&Directive<FieldPayload>, Option<&Directive<FieldPayload>>),
) -> Owes {
    let slot = member.slot;
    if !roster.member_available(slot)
        || crate::field::field_of_unit(ctx.w, member.unit).as_ref() != Some(site)
    {
        return Owes::Nothing;
    }
    let pending = book.pending(slot);
    let ours =
        pending.filter(|order| order.issuer == issuer && order.command_term == roster.term());
    let delivered = book.delivered(slot);
    // Doc 45 unit AP.8: an order that lapsed with its leader's absence does not stand, whatever
    // it said; the commander's own posture replaces it.
    let refused = delivered.is_some_and(|delivered| {
        orders.is_some_and(|orders| orders.lapse_of(slot, delivered.order.sequence).is_some())
            || delivered
                .acknowledgement
                .is_some_and(|answer| answer.response == Response::Negative)
    });
    let concentrating =
        delivered.is_some_and(|delivered| field_orders::concentrates(&delivered.order.directive));
    let lapsed = delivered.is_some_and(|delivered| {
        orders.is_some_and(|orders| orders.lapse_of(slot, delivered.order.sequence).is_some())
    });
    // Seen through a CONCENTRATE, whose own answer says nothing of the posture under it.
    let stands = match (delivered, concentrating) {
        (None, _) => false,
        (Some(_), true) => {
            !lapsed
                && orders
                    .and_then(|orders| orders.underlying(book, slot))
                    .is_some_and(|order| order.directive == *posture)
        }
        (Some(delivered), false) => delivered.order.directive == *posture && !refused,
    };
    let posture_in_flight = ours.is_some_and(|order| order.directive == *posture);
    let ours_concentrate = ours.is_some_and(|order| field_orders::concentrates(&order.directive));
    // An order in flight that is not the posture is superseded by the next issue; one that is
    // the posture is waited for, whatever stands meanwhile.
    if !posture_in_flight && ((pending.is_some() && !ours_concentrate) || !stands) {
        return Owes::Posture;
    }
    let Some(contact) = contact else {
        return Owes::Nothing;
    };
    let same = |order: &Directive<FieldPayload>| named_by(order) == named_by(contact);
    let on_its_way = ours.is_some_and(|order| same(&order.directive));
    let taken = delivered.is_some_and(|delivered| same(&delivered.order.directive)) && !refused;
    if stands
        && !on_its_way
        && !taken
        && (pending.is_none() || ours_concentrate)
        && can_fire(ctx, member.unit)
    {
        Owes::Concentrate
    } else {
        Owes::Nothing
    }
}

/// **The commander's turn on its own unit** (this module's head): `None` when `actor` is not
/// the roster's NPC leader in a running field, when its frame has not arrived, or when every
/// hull it can reach already has what it decided or is inside the resend guard; else the step
/// its one order took.
pub(super) fn step(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    sent: &mut Sent,
    (h, op, roster, flight): (&SortieHandles, &OperationRecord, &DetachmentRoster, &Flight),
    (unit, actor): (ObjId, ObjId),
) -> Option<Step> {
    let leader = roster.leader()?;
    if leader.kind != LeaderKind::Npc || leader.character != actor || op.ended.is_some() {
        return None;
    }
    let site = crate::field::field_of_unit(ctx.w, unit)?;
    let book = match op.orders.as_deref() {
        Some(orders) => OrderBook::restore(&orders.book, roster).ok()?,
        None => OrderBook::new(roster.id()),
    };
    let decision = decide(&view(ctx, (op, roster, flight), (unit, actor))?);
    let posture = directive(decision.posture);
    // The contact, while the commander's own frame holds it Hostile: a name that stands
    // through the dwell for a contact out of its sight is not ordered again meanwhile.
    let contact = decision
        .concentrate
        .and_then(|key| crate::field::orders::hostile_contact(ctx.w, unit, actor, key))
        .map(|objective| {
            Directive::Objective(FieldPayload {
                objective,
                hold_fire: false,
            })
        });
    let now = ctx.w.now();
    let reach: Vec<(&Member, Owes)> = roster
        .members()
        .iter()
        .map(|member| {
            (
                member,
                owes(
                    ctx,
                    &site,
                    roster,
                    (&book, op.orders.as_deref()),
                    actor,
                    member,
                    (&posture, contact.as_ref()),
                ),
            )
        })
        .collect();
    // The posture first, to every hull that owes it; then the contact.
    let (wanted, kind) = if reach.iter().any(|(_, owes)| *owes == Owes::Posture) {
        (posture, Owes::Posture)
    } else {
        (contact?, Owes::Concentrate)
    };
    let owing: Vec<&Member> = reach
        .iter()
        .filter(|(_, owes)| *owes == kind)
        .map(|(member, _)| *member)
        .filter(|member| {
            sent.get(&(unit, member.slot.get()))
                .is_none_or(|(order, until)| *order != wanted || *until <= now)
        })
        .collect();
    let first = owing.first()?.slot;
    // A CONCENTRATE that every hull the commander can reach still owes goes to all of them at
    // once, by the tolerant target; anything else goes one slot a fire.
    let reachable = roster
        .members()
        .iter()
        .filter(|member| {
            roster.member_available(member.slot)
                && crate::field::field_of_unit(ctx.w, member.unit).as_ref() == Some(&site)
        })
        .count();
    let together = kind == Owes::Concentrate && owing.len() > 1 && owing.len() == reachable;
    let (revision, term) = (roster.revision(), roster.term());
    let mut turn = None;
    let step = super::npc::dispatch(ctx, fire, unit, actor, |ctx| {
        let mut request = field_orders::request(
            ctx,
            h,
            op,
            actor,
            Act::Issue {
                revision,
                term,
                target: if together {
                    OrderTarget::Able
                } else {
                    OrderTarget::Slot(first)
                },
                directive: wanted.clone(),
                expires_at_ms: None,
            },
        )?;
        // Org's answer for this issuer covers every unit of the roster: the order goes to the
        // owing hulls the NPC holds a command right over, and to none when it holds none.
        let righted: Vec<Slot> = owing
            .iter()
            .filter(|member| {
                request
                    .rights
                    .iter()
                    .any(|right| right.unit == member.unit && right.allowed)
            })
            .map(|member| member.slot)
            .collect();
        let Some(slot) = righted.first().copied() else {
            turn = Some(Turn::NoRight);
            return Err("The commander holds no command right over an owing unit.".to_owned());
        };
        if !together && let Act::Issue { target, .. } = &mut request.act {
            *target = OrderTarget::Slot(slot);
        }
        runtime::submit(ctx, runtime::Command::Message(Box::new(request)))?;
        turn = Some(Turn::Sent(if together { righted } else { vec![slot] }));
        Ok(())
    });
    // The slots it sent to are left alone for a while. When it holds no right over any owing
    // hull, so is every slot it owed. An attempt that submitted nothing for another reason is
    // repeated soon, and not next fire, so its own hull flies its order meanwhile.
    let guard = |sent: &mut Sent, wait: i64| {
        for member in &owing {
            sent.insert(
                (unit, member.slot.get()),
                (wanted.clone(), now.saturating_add(wait)),
            );
        }
    };
    match (turn, step) {
        (Some(Turn::Sent(slots)), _) => {
            for slot in slots {
                sent.insert(
                    (unit, slot.get()),
                    (wanted.clone(), now.saturating_add(RESEND_AFTER_S)),
                );
            }
        }
        (Some(Turn::NoRight), _) => guard(sent, RESEND_AFTER_S),
        (None, Step::Refused) => guard(sent, RETRY_AFTER_S),
        (None, _) => {}
    }
    Some(step)
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
        for posture in [
            Posture::Advance { to: (128, 118) },
            Posture::Hold,
            Posture::Withdraw,
        ] {
            assert_eq!(posture_of(&directive(posture)), Some(posture));
        }
        assert_eq!(
            posture_of(&Directive::Objective(FieldPayload {
                objective: FieldObjective::Hold,
                hold_fire: true,
            })),
            None,
            "a HOLD FIRE is no order of the commander's"
        );
        assert_eq!(posture_of(&Directive::Cancel), None);
    }

    /// A history holding `given`, each `(slot, issuer, order, IC ms)`, numbered in order.
    fn history(given: &[(u16, u32, Directive<FieldPayload>, u64)]) -> FieldOrders {
        use mush_tactics::detachment::{Controller, DetachmentId, LeadershipPolicy};
        let roster = DetachmentRoster::new(
            DetachmentId(7),
            1,
            vec![Member {
                slot: Slot::new(1).expect("a slot"),
                unit: ObjId(101),
                controller: Controller::Npc {
                    character: ObjId(11),
                },
            }],
            Some(ObjId(11)),
            LeadershipPolicy::default(),
        )
        .expect("a roster");
        let mut orders = FieldOrders::new(&roster);
        for (n, (slot, issuer, order, at_ms)) in given.iter().enumerate() {
            orders.given.push(field_orders::Given {
                sequence: u64::try_from(n).expect("small") + 1,
                slot: Slot::new(*slot).expect("a slot"),
                issuer: ObjId(*issuer),
                directive: order.clone(),
                at_ms: *at_ms,
            });
        }
        orders
    }

    /// **Doc 44 unit MW.14c**: the history gives the posture last ordered and when its
    /// current spell began. An earlier spell of the same posture does not make it look old
    /// (which would let a Hostile passing a second later flip it); a posture re-sent is the
    /// same spell; a successor reads the commander's posture, and nobody's contact but its
    /// own; and a contact's name ends with the hold it was given under.
    #[test]
    fn mw14c_the_history_gives_the_posture_and_when_its_spell_began() {
        let advance = directive(Posture::Advance { to: (128, 118) });
        let hold = directive(Posture::Hold);
        let named = |id| {
            Directive::Objective(FieldPayload {
                objective: FieldObjective::Concentrate {
                    ordinal: 1,
                    id,
                    designator: smol_str::SmolStr::new("BK"),
                },
                hold_fire: false,
            })
        };
        let advancing = Posture::Advance { to: (128, 118) };
        assert_eq!(remembered(None, ObjId(11)), (None, None));
        assert_eq!(remembered(Some(&history(&[])), ObjId(11)), (None, None));
        // The advance reaches three hulls over three seconds: one spell, from the first.
        let mut given = vec![
            (1, 11, advance.clone(), 1_000),
            (2, 11, advance.clone(), 2_000),
            (3, 11, advance.clone(), 3_000),
        ];
        assert_eq!(
            remembered(Some(&history(&given)), ObjId(11)).0,
            Some((advancing, 1_000))
        );
        // A hold that slot 3 never gets, then the advance again to the two it reached, and
        // re-sent to slot 1 a minute on: the second spell, from its first order.
        given.extend([
            (1, 11, hold.clone(), 50_000),
            (2, 11, hold.clone(), 51_000),
            (1, 11, advance.clone(), 90_000),
            (2, 11, advance.clone(), 91_000),
            (1, 11, advance.clone(), 150_000),
        ]);
        assert_eq!(
            remembered(Some(&history(&given)), ObjId(11)).0,
            Some((advancing, 90_000)),
            "the first spell's advance does not make this one look old"
        );
        // A hold, and a contact named under it and named again: the contact's own spell.
        given.extend([
            (1, 11, hold.clone(), 200_000),
            (2, 11, hold.clone(), 201_000),
            (1, 11, named(70), 205_000),
            (2, 11, named(70), 265_000),
        ]);
        let kept = history(&given);
        assert_eq!(
            remembered(Some(&kept), ObjId(11)),
            (Some((Posture::Hold, 200_000)), Some((70, 205_000)))
        );
        assert_eq!(
            remembered(Some(&kept), ObjId(12)),
            (Some((Posture::Hold, 200_000)), None),
            "a successor inherits the posture's dwell and names its own contact"
        );
        // A second contact: the first one's orders do not date it.
        given.push((1, 11, named(71), 300_000));
        assert_eq!(
            remembered(Some(&history(&given)), ObjId(11)).1,
            Some((71, 300_000))
        );
        // The roster is ordered on: the contact's name ends with the hold.
        given.push((1, 11, advance.clone(), 400_000));
        assert_eq!(
            remembered(Some(&history(&given)), ObjId(11)),
            (Some((advancing, 400_000)), None)
        );
    }

    /// **Doc 44 unit MW.14c**: a plan's work point is the detachment's when it lies on the
    /// site the roster flew to, at 256 and at 512; one off the site, and a plan that names
    /// none, leave the default.
    #[test]
    fn mw14c_a_named_work_point_is_used_only_on_its_site() {
        assert_eq!(named_work_point(Some((140, 118)), 256), Some((140, 118)));
        assert_eq!(named_work_point(Some((255, 255)), 256), Some((255, 255)));
        assert_eq!(named_work_point(Some((300, 300)), 512), Some((300, 300)));
        assert_eq!(named_work_point(Some((300, 300)), 256), None);
        assert_eq!(named_work_point(Some((256, 0)), 256), None);
        assert_eq!(named_work_point(Some((0, 256)), 256), None);
        assert_eq!(named_work_point(None, 256), None);
        assert_eq!(
            mush_tactics::detachment::planning::MAX_WORK_POINT_CELLS,
            crate::surface::ground::MAX_SITE_CELLS,
            "a plan's bound is the widest site's"
        );
    }
}

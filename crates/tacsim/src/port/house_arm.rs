//! **A recovery house's arm** (doc 41 cut RS.6b; §4.4 "How it reaches a site, and what it does
//! there"): Tycho Mutual performing the retrieval it holds (`RetrievalStage::House`).
//!
//! On the port tick's `Arm::House` (agent and principal the house's dispatcher), one act a fire,
//! each through its production path as the dispatcher, who acts for the house
//! (`crate::port::house`'s grant) and is aboard and at the helm of its own two hulls: the lifter's
//! voyage to the casualty's site (`+ship/undock`), the crawler's launch, its drive beside her, the
//! tow, the drive to the ramp, the load into the lifter's bay, the crawler's recover, the voyage
//! home, and the berth into the home workshop, which posts the delivery and pays the contract. A
//! lease rig goes to its shed instead. **Nothing about a job's progress is stored**: every fire
//! re-derives the next act from where the casualty, the crawler and the lifter stand
//! ([`next_act`]), so a restart picks up where the world is. A refusal — a field over the site, a
//! drive under way, a full berth — waits a fire, as the yard's rigs do.

use mush_core::{CanonicalId, Ctx, ObjId, World};

use crate::port_tick::{Fire, Step};
use crate::surface::act::{Where, where_is};
use crate::surface::retrieval::{RetrievalOrder, RetrievalStage};

/// **What the house does next for one job.**
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HouseAct {
    /// Sail the lifter to the casualty's site (its venue word).
    Sail(String),
    /// The crawler down the ramp.
    Launch,
    /// Drive the crawler to a cell (`x,y`).
    Drive(String),
    /// Couple the crawler to the casualty.
    Tow,
    /// The casualty into the lifter's bay.
    Load,
    /// The crawler back up the ramp.
    Recover,
    /// Sail the lifter home (its venue word).
    Home(String),
    /// Hand the casualty over at the destination.
    Berth,
    /// Under way, mid-act, or waiting on the world: nothing this fire.
    Wait,
}

/// The cell of a centimetre pose.
const fn cell(x_cm: i64, y_cm: i64) -> (i64, i64) {
    (x_cm.div_euclid(1_000), y_cm.div_euclid(1_000))
}

/// The venue word of the port entry `entry` (what `+ship/undock` names).
fn venue_of(w: &World, entry: &CanonicalId) -> Option<String> {
    crate::port::market::ports(w)
        .into_iter()
        .find(|(id, ..)| id == entry)
        .map(|(_, def, _)| def.venue.to_string())
}

/// The entry `carrier` is docked at, if any.
fn docked_at(w: &World, carrier: ObjId) -> Option<CanonicalId> {
    match crate::port::ship::berth_of(w, carrier) {
        crate::port::ship::Berth::Docked { map, .. } => Some(map.entry.clone()),
        crate::port::ship::Berth::UnderWay | crate::port::ship::Berth::Elsewhere => None,
    }
}

/// Whether `carrier` has a passage filed and not yet launched.
fn pending(w: &World, carrier: ObjId) -> bool {
    w.module_state::<crate::store::TacsimStore>()
        .is_some_and(|store| {
            matches!(
                store.voyage_state(carrier),
                crate::port::voyage::VoyageState::Present(voyage) if voyage.pending.is_some()
            )
        })
}

/// **The next act for the house's job on `hull`**, the house's hulls `(lifter, crawler)`, home
/// port `home`.
#[must_use]
pub fn next_act(
    ctx: &Ctx<'_>,
    (lifter, crawler): (ObjId, ObjId),
    hull: ObjId,
    home: &CanonicalId,
) -> HouseAct {
    let w = &*ctx.w;
    let lifter_at = docked_at(w, lifter);
    let filed = pending(w, lifter);
    let crawler_in_bay = where_is(w, crawler) == (Where::Bay { hull: lifter });
    if where_is(w, hull) == (Where::Bay { hull: lifter }) {
        if !crawler_in_bay {
            return HouseAct::Recover;
        }
        return match lifter_at {
            Some(at) if &at == home => HouseAct::Berth,
            Some(_) if !filed => venue_of(w, home).map_or(HouseAct::Wait, HouseAct::Home),
            _ => HouseAct::Wait,
        };
    }
    let Some((site, pose)) = crate::surface::tow::ground_pose(w, hull) else {
        return HouseAct::Wait;
    };
    if lifter_at.as_ref() != Some(&site) {
        return match lifter_at {
            Some(_) if !filed && crawler_in_bay => {
                venue_of(w, &site).map_or(HouseAct::Wait, HouseAct::Sail)
            }
            _ => HouseAct::Wait,
        };
    }
    if crawler_in_bay {
        return HouseAct::Launch;
    }
    let record = crate::surface::tow::record_or_new(ctx, &site);
    if record.tows.act_of(crawler).is_some() {
        return HouseAct::Wait;
    }
    let Some((_, tug)) = crate::surface::tow::ground_pose(w, crawler) else {
        return HouseAct::Wait;
    };
    let ramp = crate::surface::tick::hull_view_berth(w, lifter)
        .map(|(_, _, ramp, _, _)| cell(ramp.x_cm, ramp.y_cm));
    if record.tows.towing(crawler).map(|op| op.casualty()) == Some(hull) {
        return match ramp {
            Some(at) if cell(tug.x_cm, tug.y_cm) == at => HouseAct::Load,
            Some((x, y)) => HouseAct::Drive(format!("{x},{y}")),
            None => HouseAct::Wait,
        };
    }
    let reach = i64::try_from(crate::surface::tow::TOW_REACH_CM).unwrap_or(i64::MAX);
    if (tug.x_cm - pose.x_cm).abs() <= reach && (tug.y_cm - pose.y_cm).abs() <= reach {
        return HouseAct::Tow;
    }
    let (cx, cy) = cell(pose.x_cm, pose.y_cm);
    HouseAct::Drive(format!("{},{cy}", cx + 1))
}

/// **The house's job**: the first Tycho retrieval whose destination is `home` and whose hull is
/// not yet handed over.
fn job(w: &World, home: &CanonicalId) -> Option<RetrievalOrder> {
    crate::surface::retrieval::orders(w)
        .into_iter()
        .find(|order| {
            order.stage == RetrievalStage::House
                && &order.destination == home
                && order.delivered.is_none()
        })
}

/// **The house arm's step** (`Arm::House`): one act of the house's one job this fire.
pub(crate) fn house_step(ctx: &mut Ctx<'_>, fire: &mut Fire, dispatcher: ObjId) -> Step {
    let home = crate::port::market::ports(ctx.w)
        .into_iter()
        .find_map(|(entry, _, map)| {
            (map?.house.as_ref()?.dispatcher == Some(dispatcher.0)).then_some(entry)
        });
    let Some(home) = home else {
        return Step::Idle;
    };
    let Some((lifter, crawler)) = crate::port::house::hulls(ctx.w, &home) else {
        return Step::Idle;
    };
    let Some(order) = job(ctx.w, &home) else {
        return Step::Idle;
    };
    let act = next_act(ctx, (lifter, crawler), order.hull, &home);
    if act == HouseAct::Wait {
        return Step::Idle;
    }
    let hull = order.hull;
    let result = fire.as_principal(ctx, dispatcher, dispatcher, |ctx| {
        perform(ctx, &act, (crawler, hull), &home)
    });
    match result {
        Ok(Ok(())) => Step::Submitted,
        Ok(Err(why)) => {
            tracing::debug!(hull = hull.0, ?act, %why, "tacsim: the house arm waits a fire");
            Step::Refused
        }
        Err(_) => Step::Deferred,
    }
}

/// One act, as the dispatcher (`ctx.actor`).
fn perform(
    ctx: &mut Ctx<'_>,
    act: &HouseAct,
    (crawler, hull): (ObjId, ObjId),
    home: &CanonicalId,
) -> Result<(), String> {
    let dispatcher = ctx.actor;
    let rig = |w: &World| {
        crate::surface::act::operator_rigs(w, dispatcher)
            .into_iter()
            .find(|rig| rig.tv == crawler)
            .ok_or_else(|| "the crawler is not the dispatcher's rig".to_owned())
    };
    let said = |e: mush_core::CmdError| e.to_string();
    match act {
        HouseAct::Sail(word) => crate::port::voyage::undock_as(ctx, word, false).map_err(said),
        HouseAct::Home(word) => crate::port::voyage::undock_as(ctx, word, true).map_err(said),
        HouseAct::Launch | HouseAct::Recover => {
            let kind = if *act == HouseAct::Launch {
                crate::surface::excursion::ActKind::Launch
            } else {
                crate::surface::excursion::ActKind::Recover
            };
            let rig = rig(ctx.w)?;
            crate::surface::act::act_cmd(ctx, &rig, kind, true, true, None).map_err(said)
        }
        HouseAct::Drive(to) => {
            let rig = rig(ctx.w)?;
            crate::surface::act::drive_cmd(ctx, &rig, to, true).map_err(said)
        }
        HouseAct::Tow => {
            let rig = rig(ctx.w)?;
            crate::surface::tow::tow_cmd(ctx, &rig, Some(&format!("#{}", hull.0)), true)
                .map_err(said)
        }
        HouseAct::Load => {
            let rig = rig(ctx.w)?;
            crate::surface::tow::load_cmd(ctx, &rig, true).map_err(said)
        }
        HouseAct::Berth => berth_delivery(ctx, hull, home),
        HouseAct::Wait => Ok(()),
    }
}

/// **The handover at home**: the dispatcher steps into the workshop room and berths her there out
/// of the lifter's bay, which posts the delivery (`crate::port::bay_work::berth`); a lease rig
/// goes to its shed and the delivery is posted by the house. The dispatcher goes back to the
/// port's host room after.
fn berth_delivery(ctx: &mut Ctx<'_>, hull: ObjId, home: &CanonicalId) -> Result<(), String> {
    let dispatcher = ctx.actor;
    if crate::port::tv::is_lease_rig(ctx.w, hull) {
        if crate::port::tv::hand_back(ctx, hull) {
            let _ = crate::surface::retrieval::deliver(ctx, hull, home, dispatcher);
            return Ok(());
        }
        return Err("the shed will not take her back".to_owned());
    }
    let (room, host) = {
        let (entry, def, map) = crate::port::market::ports(ctx.w)
            .into_iter()
            .find(|(id, ..)| id == home)
            .ok_or("the home port is not on the lane")?;
        let room = crate::port::workshop::room(ctx.w, &entry, def)?
            .ok_or("the home port has no workshop")?;
        (room, map.and_then(|map| map.host).map(ObjId))
    };
    ctx.w.move_to(dispatcher, room).map_err(|e| e.to_string())?;
    let berthed = crate::port::bay_work::berth(ctx, hull);
    if let Some(host) = host {
        let _ = ctx.w.move_to(dispatcher, host);
    }
    berthed.map(|_| ())
}

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
    /// Doc 41 cut RS.10: ask the landholder of the wreck's block (its site and cell) for consent,
    /// by the registry's notice.
    Ask(CanonicalId, (i64, i64)),
    /// Doc 41 cut RS.6c-1: title an ownerless wreck to the dispatcher before the tow.
    Title,
    /// Doc 41 cut RS.6c-1: file the house's salvage claim on a swept hull in its custody.
    Claim,
    /// Doc 41 cut RS.6c-1: break a swept hull up at the home workshop (IN.8).
    Scrap,
    /// Doc 41 cut RS.6c-1: keep a swept hull for resale: her title to the house.
    Keep,
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

/// Whether `carrier` has a passage filed and not yet launched, or a sailing plan still running
/// (the house's sail, [`HouseAct::Sail`], is a plan the order arm carries out).
fn pending(w: &World, carrier: ObjId) -> bool {
    let filed = w
        .module_state::<crate::store::TacsimStore>()
        .is_some_and(|store| {
            matches!(
                store.voyage_state(carrier),
                crate::port::voyage::VoyageState::Present(voyage) if voyage.pending.is_some()
            )
        });
    filed
        || matches!(
            crate::port::orders::orders_state(w, carrier),
            crate::port::orders::OrdersState::Present(orders) if orders.cursor.is_some()
        )
}

/// **The float a sailing tops the dispatcher's account up to** (doc 41 cut RS.6b), minor units:
/// 20,000 cr, provisional — a Ceres site's round trip in reaction mass and provisions, with room.
pub const FLOAT_MINOR: i64 = 2_000_000;

/// **The house's sail** ([`HouseAct::Sail`]): the dispatcher's float topped up from the house's
/// own account (the dispatcher acts for the house), then a sailing plan filed as the dispatcher —
/// fill the tank and the locker, then depart for `word` — which the order arm carries out as the
/// plan's author, buying at this port's counters like any captain's plan.
fn sail(ctx: &mut Ctx<'_>, lifter: ObjId, word: &str) -> Result<(), String> {
    let dispatcher = ctx.actor;
    // The dispatcher boards: a plan's buys need its author aboard the docked hull (the market's
    // presence, `Presence::Ship`), and it sails with her.
    let airlock = crate::port::ship::hull_view(ctx.w, lifter)
        .and_then(|view| view.airlock())
        .ok_or("the lifter has no airlock")?;
    if ctx.w.get(dispatcher).and_then(|o| o.location) != Some(airlock) {
        ctx.w
            .move_to(dispatcher, airlock)
            .map_err(|e| e.to_string())?;
    }
    let (house_account, float) = crate::port::market::ports(ctx.w)
        .into_iter()
        .find_map(|(_, def, map)| {
            let map = map?;
            let row = map.house.as_ref()?;
            (row.dispatcher == Some(dispatcher.0)).then(|| {
                let operator = def.recovery.as_ref().map(|r| r.operator.clone());
                let house = operator
                    .and_then(|op| map.orgs.get(&op))
                    .and_then(|org| org.account);
                (house, row.account)
            })
        })
        .ok_or("the dispatcher has no house")?;
    let (Some(house_account), Some(float)) = (house_account, float) else {
        return Err("the house's accounts are not open yet".to_owned());
    };
    let available = crate::port::credit::own_account(ctx.w, dispatcher)
        .and_then(|a| a.available())
        .map_or(0, |a| a.minor());
    if available < FLOAT_MINOR {
        let short = FLOAT_MINOR - available;
        let mut plan = mush_economy::EconomyPlan::new(
            mush_economy::IdempotencyKey::new(
                "tacsim.house.float",
                format!("{float}/{}", ctx.w.world_ic_now().at),
            ),
            mush_org::PrincipalId::Object(dispatcher),
            mush_economy::EconomicCause::Internal {
                kind: smol_str::SmolStr::new_static("tacsim.house.float"),
                id: float,
            },
            vec![
                mush_economy::RequestedLine {
                    account: mush_economy::AccountId(house_account),
                    delta: mush_economy::Amount::from_minor(-short),
                    memo: "float".into(),
                },
                mush_economy::RequestedLine {
                    account: mush_economy::AccountId(float),
                    delta: mush_economy::Amount::from_minor(short),
                    memo: "float".into(),
                },
            ],
        );
        plan.description = "the house's sailing float".to_owned();
        mush_economy::post(ctx, &plan).map_err(|e| format!("the float did not post: {e}"))?;
    }
    for line in [
        "buy propellant fill".to_owned(),
        "buy provisions fill".to_owned(),
        format!("depart {word}"),
    ] {
        let parsed = mush_core::ParsedArgs::parse(mush_core::ArgShape::Raw, &line);
        let old = std::mem::replace(&mut ctx.args, parsed);
        let filed = crate::port::orders::order_cmd(ctx);
        ctx.args = old;
        filed.map_err(|e| e.to_string())?;
    }
    Ok(())
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

/// **The sweep's next act** (doc 41 cut RS.6c-1, [`crate::port::house_sweep`]): a swept hull the
/// house holds (in the lifter's bay, or berthed in the home workshop) first, else the next wreck
/// to sweep. `None` when there is nothing to sweep.
fn sweep_act(
    ctx: &Ctx<'_>,
    (lifter, crawler): (ObjId, ObjId),
    home: &CanonicalId,
) -> Option<(ObjId, HouseAct)> {
    use crate::port::house_sweep::{Disposition, assess, held, target, title_of};
    let workshop = crate::port::market::ports(ctx.w)
        .into_iter()
        .find(|(id, ..)| id == home)
        .and_then(|(entry, def, _)| {
            crate::port::workshop::room(ctx.w, &entry, def)
                .ok()
                .flatten()
        });
    let dispatcher = crate::port::coop::helm_holder(ctx.w, lifter)?;
    if let Some(hull) = held(ctx, (lifter, crawler), workshop) {
        let custody = crate::surface::salvage::custody_of(ctx, hull);
        let berthed = workshop.is_some() && ctx.w.get(hull).and_then(|o| o.location) == workshop;
        if !berthed {
            if custody
                .as_ref()
                .is_some_and(|row| row.claim == crate::surface::salvage::ClaimSeam::Owed)
            {
                return Some((hull, HouseAct::Claim));
            }
            return Some((hull, next_act(ctx, (lifter, crawler), hull, home)));
        }
        // Berthed at home: the claim must be awarded before anything is done with her.
        if title_of(ctx.w, hull) != Some(mush_org::PrincipalId::Object(dispatcher)) {
            return Some((hull, HouseAct::Wait));
        }
        return Some((
            hull,
            match assess(ctx.w, hull) {
                Disposition::Keep { .. } => HouseAct::Keep,
                Disposition::Scrap { .. } => HouseAct::Scrap,
            },
        ));
    }
    let (hull, site, cell, consent) = target(ctx, dispatcher)?;
    // Doc 41 cut RS.10: on a block nobody has asked, the registry's notice first.
    if consent == crate::surface::claim::Consent::NotAsked {
        return Some((hull, HouseAct::Ask(site, cell)));
    }
    if title_of(ctx.w, hull).is_none() {
        return Some((hull, HouseAct::Title));
    }
    Some((hull, next_act(ctx, (lifter, crawler), hull, home)))
}

/// **The house's accounts and root scope** for `dispatcher`: `(float, house account, org,
/// root scope)`.
fn house_accounts(
    w: &World,
    dispatcher: ObjId,
) -> Option<(u64, u64, mush_org::OrgId, mush_org::OrgScopeId)> {
    crate::port::market::ports(w)
        .into_iter()
        .find_map(|(_, def, map)| {
            let map = map?;
            let row = map.house.as_ref()?;
            if row.dispatcher != Some(dispatcher.0) {
                return None;
            }
            let org = map.orgs.get(&def.recovery.as_ref()?.operator)?;
            Some((
                row.account?,
                org.account?,
                mush_org::OrgId(org.org),
                mush_org::OrgScopeId(org.root_scope),
            ))
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
    // The house's job: a retrieval it holds, else (doc 41 cut RS.6c-1) the sweep.
    let Some((hull, act)) = job(ctx.w, &home)
        .map(|order| {
            (
                order.hull,
                next_act(ctx, (lifter, crawler), order.hull, &home),
            )
        })
        .or_else(|| sweep_act(ctx, (lifter, crawler), &home))
    else {
        return Step::Idle;
    };
    if act == HouseAct::Wait {
        return Step::Idle;
    }
    let result = fire.as_principal(ctx, dispatcher, dispatcher, |ctx| {
        perform(ctx, &act, (lifter, crawler, hull), &home)
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
    (lifter, crawler, hull): (ObjId, ObjId, ObjId),
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
        HouseAct::Sail(word) => sail(ctx, lifter, word),
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
        HouseAct::Ask(site, cell) => crate::surface::claim::ask_consent_at(ctx, site, *cell),
        HouseAct::Title => crate::port::house_sweep::title_ownerless(ctx, hull),
        HouseAct::Claim => {
            crate::surface::claim::tow_files_salvage(ctx, &format!("#{}", hull.0)).map_err(said)
        }
        HouseAct::Keep => {
            let (_, _, org, scope) =
                house_accounts(ctx.w, dispatcher).ok_or("the house's accounts are not open")?;
            crate::port::house_sweep::to_house(ctx, hull, (org, scope))
        }
        HouseAct::Scrap => scrap_at_home(ctx, hull, home),
        HouseAct::Wait => Ok(()),
    }
}

/// **Break a swept hull up at the home workshop** (doc 41 cut RS.6c-1; IN.8's `scrap`): the
/// dispatcher steps into the workshop room, scraps her as her title holder, steps back, and the
/// scrap's credit (paid into its float) goes to the house's account.
fn scrap_at_home(ctx: &mut Ctx<'_>, hull: ObjId, home: &CanonicalId) -> Result<(), String> {
    let dispatcher = ctx.actor;
    let (room, host) = {
        let (entry, def, map) = crate::port::market::ports(ctx.w)
            .into_iter()
            .find(|(id, ..)| id == home)
            .ok_or("the home port is not on the lane")?;
        let room = crate::port::workshop::room(ctx.w, &entry, def)?
            .ok_or("the home port has no workshop")?;
        (room, map.and_then(|map| map.host).map(ObjId))
    };
    let back = ctx.w.get(dispatcher).and_then(|o| o.location).or(host);
    ctx.w.move_to(dispatcher, room).map_err(|e| e.to_string())?;
    let scrapped = crate::port::bay_work::strip::scrap(ctx, hull);
    if let Some(back) = back {
        let _ = ctx.w.move_to(dispatcher, back);
    }
    let credit = scrapped?.credit_minor;
    let (float, house, _, _) =
        house_accounts(ctx.w, dispatcher).ok_or("the house's accounts are not open")?;
    crate::port::house_sweep::return_surplus(ctx, (float, house), credit)
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

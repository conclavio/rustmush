//! **A recovery operator's house** (doc 41 cut RS.6a; §4.4, Q-41-15, Q-41-17).
//!
//! Tycho Mutual's two house hulls — a `tycho_lifter` docked at Ceres and a `tycho_crawler` in its
//! bay — are the registered designs anyone may build at the Stickney yard (Q-41-15), so the house's
//! own pair is laid up the way a built hull is: as two **yard commissions** titled to the
//! operator, which the commission arm (`crate::port::commission`) carries from `Laying` through
//! the placement, the title, the hold, the provenance and the register. A house seed has no kit
//! lot and no work order (both `0` on its provenance). The arm here, on the port tick before the
//! commission arm, makes in order — each step idempotent, the port map written after each:
//!
//! 1. the house's `DARK` dispatcher in the port's host room (the Occator Yard's precedent): the
//!    object that acts for the house, the commissions' requester;
//! 2. the lifter's commission, a ship docked at the port;
//! 3. the crawler's commission, a rig on the workshop floor;
//! 4. once both are done, the crawler stowed into the lifter's bay.
//!
//! The lane authors it as `recovery.house` (lane v28): the two marks. RS.6b's arm flies them.

use mush_core::{CanonicalId, Ctx, ObjId};
use mush_org::{OrgId, PrincipalId};

use crate::port::commission::{Step, YardCommission};
use crate::port::def::{PortDef, RecoveryHouseDef};
use crate::port::map::{HouseRow, PortMap};
use crate::store::put_port_map;

/// The dispatcher's name.
pub const DISPATCHER_NAME: &str = "Tycho Mutual dispatcher";

/// **The house arm** (the port tick, before the commission arm): every port whose recovery row
/// authors a house is carried one step nearer laid up. A failure is logged and retried next fire.
pub(crate) fn tick(ctx: &mut Ctx<'_>) {
    let houses: Vec<(CanonicalId, PortDef, PortMap)> = crate::port::market::ports(ctx.w)
        .into_iter()
        .filter_map(|(entry, def, map)| {
            let map = map?;
            def.recovery.as_ref()?.house.as_ref()?;
            (!map.house.as_ref().is_some_and(|row| row.stowed))
                .then(|| (entry, def.clone(), map.clone()))
        })
        .collect();
    for (entry, def, map) in houses {
        if let Err(error) = step(ctx, &entry, &def, map) {
            tracing::warn!(port = %entry, %error, "tacsim: a recovery house step failed; it resumes on the next fire");
        }
    }
}

/// The house row of `map`, made when absent.
fn row(map: &mut PortMap) -> &mut HouseRow {
    map.house.get_or_insert_with(HouseRow::default)
}

/// One port's house, carried as far as it goes this fire.
fn step(
    ctx: &mut Ctx<'_>,
    entry: &CanonicalId,
    def: &PortDef,
    mut map: PortMap,
) -> Result<(), String> {
    let recovery = def.recovery.as_ref().ok_or("no recovery row")?;
    let house: &RecoveryHouseDef = recovery.house.as_deref().ok_or("no house")?;
    let operator = map
        .orgs
        .get(&recovery.operator)
        .map(|org| PrincipalId::Organization(OrgId(org.org)))
        .ok_or_else(|| format!("{}: the operator has no map row yet", recovery.operator))?;
    // 1. The dispatcher.
    let dispatcher = match map.house.as_ref().and_then(|row| row.dispatcher) {
        Some(thing) => ObjId(thing),
        None => {
            let host = map.host.map(ObjId).ok_or("the port has no host room yet")?;
            let thing = crate::port::site::dark_thing(ctx, entry, DISPATCHER_NAME, host)?;
            row(&mut map).dispatcher = Some(thing);
            put_port_map(ctx, &map);
            ObjId(thing)
        }
    };
    // 2–3. The commissions.
    let designs = [
        (
            true,
            crate::definitions::content::TYCHO_LIFTER,
            &house.lifter_mark,
        ),
        (
            false,
            crate::definitions::content::TYCHO_CRAWLER,
            &house.crawler_mark,
        ),
    ];
    for (ship, design, mark) in designs {
        let seeded = map
            .house
            .as_ref()
            .and_then(|row| if ship { row.lifter } else { row.crawler });
        if seeded.is_some() {
            continue;
        }
        let order = crate::port::apply::alloc_vault_id(ctx)?.0;
        // The house's own pool indices, past every yard order's (its marks are its own).
        let n = crate::port::commission::HOUSE_POOL_BASE + u32::from(!ship);
        let record = YardCommission {
            order,
            work_order: 0,
            facility: dispatcher,
            home: entry.clone(),
            design: CanonicalId::new(design).map_err(|_| "a design id".to_owned())?,
            requester: dispatcher,
            beneficiary: operator.clone(),
            mark: mark.clone(),
            n,
            ship,
            kit: mush_inventory::LotId(0),
            parts: Vec::new(),
            step: Step::Laying,
            row: None,
            hold: None,
            at_ic: ctx.w.world_ic_now().at,
        };
        crate::port::commission::put(ctx, &record);
        if ship {
            row(&mut map).lifter = Some(order);
        } else {
            row(&mut map).crawler = Some(order);
        }
        put_port_map(ctx, &map);
    }
    // 4. Both laid up: the crawler into the lifter's bay.
    let done = |order: Option<u64>| {
        order
            .and_then(|order| crate::port::commission::commission(ctx.w, order))
            .filter(|record| record.step == Step::Done)
            .and_then(YardCommission::object)
    };
    let row_now = map.house.clone().unwrap_or_default();
    let (Some(lifter), Some(crawler)) = (done(row_now.lifter), done(row_now.crawler)) else {
        return Ok(());
    };
    crate::port::bay_work::stow_house(ctx, crawler, lifter)?;
    row(&mut map).stowed = true;
    put_port_map(ctx, &map);
    Ok(())
}

/// **The house's hulls at `entry`**, once laid up: `(lifter, crawler)`.
#[must_use]
pub fn hulls(w: &mush_core::World, entry: &CanonicalId) -> Option<(ObjId, ObjId)> {
    let map = crate::port::market::ports(w)
        .into_iter()
        .find(|(id, ..)| id == entry)?
        .2?;
    let row = map.house.as_ref()?;
    let object = |order: Option<u64>| {
        crate::port::commission::commission(w, order?).and_then(YardCommission::object)
    };
    Some((object(row.lifter)?, object(row.crawler)?))
}

/// **The house's dispatcher at `entry`**, once made.
#[must_use]
pub fn dispatcher(w: &mush_core::World, entry: &CanonicalId) -> Option<ObjId> {
    crate::port::market::ports(w)
        .into_iter()
        .find(|(id, ..)| id == entry)?
        .2?
        .house
        .as_ref()?
        .dispatcher
        .map(ObjId)
}

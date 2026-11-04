//! **A port's refinery and works** (doc 39 cut IN.5; §4.2's facility table, Q-39-4, Q-39-5): the
//! rooms a port entry authors as its `refinery` and `works` are opened, by the `PortApplier`'s
//! parts l and m, as industry facilities of their operators — GC.7's workshop precedent
//! ([`crate::port::workshop`]): the room is the facility object, a facility already on it must be
//! the one the entry would open, and a changed cost model or customer policy is re-termed in
//! place (`mush_industry::store::install_authored_facility`), never drift.
//!
//! - **The refinery** holds `smelt` and its cost model, one slot never booked: the refine itself
//!   is the vendor's `kind: refine` mill row on the stock clerk's turn (Q-39-6).
//! - **The works** holds the works capability `+works` finds it by
//!   ([`crate::port::works::WORKS_CAPABILITY`]), its slots, its cost model (the fee and the
//!   booking fee, IN.4) and its customers (IN.3's `public`).

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_industry::{Customers, FacilityCostDef, FacilityRecord, IndustryStore};
use mush_org::{OrgId, PrincipalId};
use std::collections::BTreeSet;

use crate::port::apply::room_by_key;
use crate::port::def::{PortDef, room_key};
use crate::port::map::PortMap;

/// One authored facility row, whichever block it came from.
struct Row<'d> {
    block: &'static str,
    room: &'d str,
    operator: &'d CanonicalId,
    capabilities: &'d BTreeSet<CanonicalId>,
    slots: u32,
    cost: FacilityCostDef,
    customers: Customers,
}

/// The entry's refinery and works rows, in apply order.
fn rows(def: &PortDef) -> Vec<Row<'_>> {
    let mut rows = Vec::new();
    if let Some(refinery) = &def.refinery {
        rows.push(Row {
            block: "refinery",
            room: &refinery.room,
            operator: &refinery.operator,
            capabilities: &refinery.capabilities,
            slots: 1,
            cost: refinery.cost,
            customers: Customers::OwnerOnly,
        });
    }
    if let Some(works) = &def.works {
        rows.push(Row {
            block: "works",
            room: &works.room,
            operator: &works.operator,
            capabilities: &works.capabilities,
            slots: works.slots,
            cost: works.cost,
            customers: works.customers,
        });
    }
    rows
}

/// The row's room on this world.
fn room_of(w: &World, id: &CanonicalId, def: &PortDef, row: &Row<'_>) -> Result<ObjId, String> {
    let key = room_key(&def.area, row.room).map_err(|detail| format!("{id}: {detail}"))?;
    room_by_key(w, &key).ok_or_else(|| {
        format!(
            "{id}: the {} room {} is not on this world",
            row.block, row.room
        )
    })
}

/// The works room's object on this world, when the entry authors one.
///
/// # Errors
///
/// A room reference outside the port's area, or one no room carries.
pub fn works_room(w: &World, id: &CanonicalId, def: &PortDef) -> Result<Option<ObjId>, String> {
    rows(def)
        .iter()
        .find(|row| row.block == "works")
        .map(|row| room_of(w, id, def, row))
        .transpose()
}

/// The facility record a row's room carries, owned by `operator`.
fn record(room: ObjId, operator: OrgId, row: &Row<'_>) -> Result<FacilityRecord, String> {
    FacilityRecord::new(
        room,
        PrincipalId::Organization(operator),
        row.capabilities.clone(),
        row.slots,
    )
    .and_then(|record| record.with_authored_cost(row.cost))
    .map(|record| record.with_authored_customers(row.customers))
    .map_err(|error| error.to_string())
}

/// **Parts l and m's refusals before any write**: each room resolves, and a facility already on
/// it is exactly the one this entry would open (its operator's org, from a present map). A
/// changed slot count or capability set is drift, refused.
///
/// # Errors
///
/// The sentence naming the entry and what is wrong.
pub(crate) fn preflight(
    w: &World,
    id: &CanonicalId,
    def: &PortDef,
    map: Option<&PortMap>,
) -> Result<(), String> {
    for row in rows(def) {
        let room = room_of(w, id, def, &row)?;
        let Some(existing) = w
            .module_state::<IndustryStore>()
            .and_then(|store| store.facility(room))
        else {
            continue;
        };
        let expected = map
            .and_then(|m| m.orgs.get(row.operator))
            .map(|org| record(room, OrgId(org.org), &row))
            .transpose()?;
        if !expected
            .as_ref()
            .is_some_and(|expected| expected.same_identity(existing))
        {
            return Err(format!(
                "{id}: the {} room {} is already a facility this entry did not open as \
                 authored (its owner, capabilities or slots differ); repair it by hand",
                row.block, row.room
            ));
        }
    }
    Ok(())
}

/// **Parts l and m**: open the refinery and the works rooms as their operators' facilities, once,
/// re-terming a changed cost or customer policy. After the institutions (part c), whose orgs own
/// them.
///
/// # Errors
///
/// The sentence naming the entry and what is wrong.
pub(crate) fn put(
    ctx: &mut Ctx<'_>,
    id: &CanonicalId,
    def: &PortDef,
    map: &PortMap,
) -> Result<(), String> {
    for row in rows(def) {
        let room = room_of(ctx.w, id, def, &row)?;
        let operator = map
            .orgs
            .get(row.operator)
            .map(|org| OrgId(org.org))
            .ok_or_else(|| {
                format!(
                    "{id}: {}: no org is recorded for {}",
                    row.block, row.operator
                )
            })?;
        mush_industry::store::install_authored_facility(ctx, record(room, operator, &row)?)
            .map_err(|error| format!("{id}: {}: {error}", row.block))?;
    }
    Ok(())
}

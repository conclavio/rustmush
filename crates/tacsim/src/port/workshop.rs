//! **A port's workshop** (doc 36 cut GC.7; Q-36-53's rooms, Q-36-31's tariff): the room a port
//! entry authors as its workshop is opened, by the `PortApplier`'s part j, as an industry
//! facility of the operator org holding the `bay_refit` capability with the authored berths as
//! its slots; and a quote at it is the shipped workshop tariff
//! ([`crate::definitions::workshop`]) at the operator's authored multiplier.
//!
//! The room is the facility object itself, so a hull whose location is the room is at the
//! facility, which is what the bay's authority check reads (`bay_refit/authority.rs`). Moving a
//! hull from a docked carrier's bay into the room (`berth`) and back (`stow`), the repair
//! actions and `+bay` are GC.8's; titling the room to the operator's scope and composing the
//! bay's payment site over it come with them.

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_industry::{FacilityRecord, IndustryStore};
use mush_org::{OrgId, PrincipalId};
use mush_tactics::TacticalUnit;
use mush_tactics::field_ops::WorkshopCostQuote;

use crate::definitions::unit::UnitSpec;
use crate::port::apply::room_by_key;
use crate::port::def::{PortDef, WorkshopDef, room_key};
use crate::port::map::PortMap;

/// The workshop room's object on this world, when the entry authors one.
///
/// # Errors
///
/// A room reference outside the port's area, or one no room carries.
pub fn room(w: &World, id: &CanonicalId, def: &PortDef) -> Result<Option<ObjId>, String> {
    let Some(workshop) = &def.workshop else {
        return Ok(None);
    };
    let key = room_key(&def.area, &workshop.room).map_err(|detail| format!("{id}: {detail}"))?;
    room_by_key(w, &key).map(Some).ok_or_else(|| {
        format!(
            "{id}: the workshop room {} is not on this world",
            workshop.room
        )
    })
}

/// The facility record the workshop's room carries, owned by `operator`.
fn record(room: ObjId, operator: OrgId, workshop: &WorkshopDef) -> Result<FacilityRecord, String> {
    FacilityRecord::new(
        room,
        PrincipalId::Organization(operator),
        workshop.capabilities.clone(),
        workshop.berths,
    )
    .map_err(|error| error.to_string())
}

/// **Part j's refusals before any write**: the room resolves, and a facility already on it is
/// exactly the one this entry would open (its operator's org, from a present map). A changed
/// berth count or capability set is drift, refused, never rewritten under hulls in the room.
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
    let (Some(workshop), Some(room)) = (&def.workshop, room(w, id, def)?) else {
        return Ok(());
    };
    let Some(existing) = w
        .module_state::<IndustryStore>()
        .and_then(|store| store.facility(room))
    else {
        return Ok(());
    };
    let expected = map
        .and_then(|m| m.orgs.get(&workshop.operator))
        .map(|row| record(room, OrgId(row.org), workshop))
        .transpose()?;
    if expected.as_ref() != Some(existing) {
        return Err(format!(
            "{id}: the workshop room {} is already a facility this entry did not open as \
             authored (its owner, capabilities or berths differ); repair it by hand",
            workshop.room
        ));
    }
    Ok(())
}

/// **Part j**: open the workshop room as the operator's `bay_refit` facility, once. After the
/// institutions (part c), whose org owns it.
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
    let (Some(workshop), Some(room)) = (&def.workshop, room(ctx.w, id, def)?) else {
        return Ok(());
    };
    let operator = map
        .orgs
        .get(&workshop.operator)
        .map(|row| OrgId(row.org))
        .ok_or_else(|| {
            format!(
                "{id}: workshop: no org is recorded for {}",
                workshop.operator
            )
        })?;
    mush_industry::store::install_authored_facility(ctx, record(room, operator, workshop)?)
        .map_err(|error| format!("{id}: workshop: {error}"))
}

/// **A quote at an authored workshop** (doc 36 Q-36-31): the shipped tariff at the workshop's
/// multiplier, over `unit` as `spec` describes it. Labour only: the quote names the tonnes of
/// `industrial_parts` the points consume, and rounds come from munitions goods the customer
/// brings (Q-36-36).
///
/// # Errors
///
/// The tariff's or the engine's refusal, as a sentence.
pub fn quote(
    workshop: &WorkshopDef,
    unit: &TacticalUnit,
    spec: &UnitSpec,
) -> Result<WorkshopCostQuote, String> {
    let tariff = crate::definitions::workshop::workshop_tariff()
        .map_err(|error| error.to_string())?
        .with_multiplier_permille(workshop.multiplier_permille)
        .map_err(|error| error.to_string())?;
    let anatomy = crate::field_ops::anatomy_for_unit_spec(spec).map_err(|e| e.to_string())?;
    mush_tactics::field_ops::WorkshopBook::new()
        .quote_full_restoration(unit, &anatomy, &tariff)
        .map_err(|error| error.to_string())
}

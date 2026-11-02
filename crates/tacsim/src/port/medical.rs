//! **A port's medical room** (doc 36 cut GC.13; Q-36-45): the room a port entry authors as its
//! medical room is opened, by the `PortApplier`'s part k, as an industry facility of the
//! operator org holding the [`MEDICAL_CAPABILITY`](crate::port::def::MEDICAL_CAPABILITY) with
//! the authored beds as its slots — the way GC.7's workshop room is (`crate::port::workshop`).
//! Crew treatment (`crate::field::crew`) reads it back off the industry store: a port with a
//! facility holding the capability in one of its rooms treats faster.

use mush_core::{CanonicalId, Ctx, ObjId, World};
use mush_industry::{FacilityRecord, IndustryStore};
use mush_org::{OrgId, PrincipalId};

use crate::port::apply::room_by_key;
use crate::port::def::{MEDICAL_CAPABILITY, MedicalDef, PortDef, room_key};
use crate::port::map::PortMap;

/// The medical room's object on this world, when the entry authors one.
///
/// # Errors
///
/// A room reference outside the port's area, or one no room carries.
pub fn room(w: &World, id: &CanonicalId, def: &PortDef) -> Result<Option<ObjId>, String> {
    let Some(medical) = &def.medical else {
        return Ok(None);
    };
    let key = room_key(&def.area, &medical.room).map_err(|detail| format!("{id}: {detail}"))?;
    room_by_key(w, &key).map(Some).ok_or_else(|| {
        format!(
            "{id}: the medical room {} is not on this world",
            medical.room
        )
    })
}

/// The facility record the medical room carries, owned by `operator`.
fn record(room: ObjId, operator: OrgId, medical: &MedicalDef) -> Result<FacilityRecord, String> {
    FacilityRecord::new(
        room,
        PrincipalId::Organization(operator),
        medical.capabilities.clone(),
        medical.beds,
    )
    .map_err(|error| error.to_string())
}

/// **Part k's refusals before any write**: the room resolves, and a facility already on it is
/// exactly the one this entry would open (its operator's org, from a present map).
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
    let (Some(medical), Some(room)) = (&def.medical, room(w, id, def)?) else {
        return Ok(());
    };
    let Some(existing) = w
        .module_state::<IndustryStore>()
        .and_then(|store| store.facility(room))
    else {
        return Ok(());
    };
    let expected = map
        .and_then(|m| m.orgs.get(&medical.operator))
        .map(|row| record(room, OrgId(row.org), medical))
        .transpose()?;
    if expected.as_ref() != Some(existing) {
        return Err(format!(
            "{id}: the medical room {} is already a facility this entry did not open as \
             authored (its owner, capabilities or beds differ); repair it by hand",
            medical.room
        ));
    }
    Ok(())
}

/// **Part k**: open the medical room as the operator's `medical` facility, once. After the
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
    let (Some(medical), Some(room)) = (&def.medical, room(ctx.w, id, def)?) else {
        return Ok(());
    };
    let operator = map
        .orgs
        .get(&medical.operator)
        .map(|row| OrgId(row.org))
        .ok_or_else(|| format!("{id}: medical: no org is recorded for {}", medical.operator))?;
    mush_industry::store::install_authored_facility(ctx, record(room, operator, medical)?)
        .map_err(|error| format!("{id}: medical: {error}"))
}

/// **Whether `port` has a medical room**: a facility holding the
/// [`MEDICAL_CAPABILITY`] whose room is in that port.
#[must_use]
pub fn has_medical_room(w: &World, port: &crate::ic_return::IcPort) -> bool {
    w.module_state::<IndustryStore>().is_some_and(|store| {
        store.facilities().into_iter().any(|facility| {
            facility
                .capabilities()
                .iter()
                .any(|word| word.as_str() == MEDICAL_CAPABILITY)
                && crate::ic_return::port_of_room(w, facility.object())
                    .is_some_and(|at| at.id == port.id)
        })
    })
}

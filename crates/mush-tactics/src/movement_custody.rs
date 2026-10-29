//! Bounded actual-owner custody facts for Sim movement. Opaque unit cells,
//! checkout snapshots and historical custody records never leave this service.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::read_service::{
    OwnerReadContext, OwnerReadError, OwnerReadHandle, OwnerReadService,
};
use mush_core::{DefinitionRegistryBuilder, MechanicalState, ModError, ObjId, OwnerId, Registrar};
use mush_sim::movement_custody::{
    MAX_MOVEMENT_CUSTODY_BYTES, MAX_MOVEMENT_CUSTODY_OBJECTS, MAX_MOVEMENT_CUSTODY_RECORDS,
    MovementCustodyFact, MovementCustodyFacts, MovementCustodyRequest,
};
use serde_json::json;

use crate::module::UnitStore;

pub struct MovementCustodyReader;
impl MovementCustodyReader {
    pub fn request(reg: &mut Registrar<'_>) -> Result<OwnerReadHandle<Self>, ModError> {
        let definition = DefinitionRegistryBuilder::new()
            .register(
                OwnerId::new(crate::module::NS).map_err(bad)?,
                mush_sim::movement_custody::descriptor()?,
            )
            .map_err(bad)?;
        reg.request_owner_read::<Self>(definition)
    }
}
fn bad(e: impl std::fmt::Display) -> ModError {
    ModError::msg(e.to_string())
}
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    let descriptor = mush_sim::movement_custody::descriptor()?;
    let owner = OwnerId::new(crate::module::NS).map_err(bad)?;
    let expected = DefinitionRegistryBuilder::new()
        .register(owner, descriptor.clone())
        .map_err(bad)?;
    reg.definition_validator(
        descriptor.kind().clone(),
        move |ctx: &mush_core::DefinitionValidationContext<'_>| {
            if ctx.definition().reference() == &expected && !ctx.definition().is_current() {
                Ok(())
            } else {
                Err("unknown Tactics movement custody read".into())
            }
        },
    )?;
    let definition = reg.definition(descriptor)?;
    reg.owner_read_service(
        definition,
        BTreeSet::from([OwnerId::new("sim").map_err(bad)?]),
        BTreeSet::new(),
        MovementCustodyReader,
    )
}

fn charge(text: &str, budget: &mut usize) -> Result<(), OwnerReadError> {
    if text.len() > 1024 {
        return Err(OwnerReadError::Limit);
    }
    *budget = budget
        .checked_sub(text.len())
        .ok_or(OwnerReadError::Limit)?;
    Ok(())
}
fn derive(
    store: &UnitStore,
    objects: &BTreeSet<ObjId>,
) -> Result<MovementCustodyFacts, OwnerReadError> {
    if objects.is_empty()
        || objects.len() > MAX_MOVEMENT_CUSTODY_OBJECTS
        || store.records.len() > MAX_MOVEMENT_CUSTODY_RECORDS
    {
        return Err(OwnerReadError::Limit);
    }
    let schema = store
        .record_definition
        .as_ref()
        .ok_or(OwnerReadError::Unavailable)?;
    let ic = store
        .ic
        .movement_preimage(objects)
        .map_err(|_| OwnerReadError::Integrity)?;
    let mut facts: BTreeMap<_, _> = objects
        .iter()
        .map(|object| {
            (
                *object,
                MovementCustodyFact {
                    object: *object,
                    active_unit: store.units.contains_key(object),
                    held_record: false,
                    ooc_deployed: store.deployed.contains_key(object),
                    ic_custodied: store.is_ic_custodied(*object),
                },
            )
        })
        .collect();
    let mut claims = BTreeMap::<ObjId, usize>::new();
    let mut records = Vec::new();
    let mut text = 64 * 1024;
    for row in &store.records {
        let selected: BTreeSet<_> = [row.key_object, row.object]
            .into_iter()
            .flatten()
            .filter(|id| objects.contains(id))
            .collect();
        if selected.is_empty() {
            continue;
        }
        if records.len() >= MAX_MOVEMENT_CUSTODY_OBJECTS {
            return Err(OwnerReadError::Limit);
        }
        charge(&row.key, &mut text)?;
        if let Some(marker) = &row.deployed {
            charge(&marker.mark.arena, &mut text)?;
        }
        let active = row.state == MechanicalState::Active;
        for object in selected {
            let fact = facts.get_mut(&object).ok_or(OwnerReadError::Integrity)?;
            let count = claims.entry(object).or_default();
            *count += 1;
            fact.held_record |= !active
                || row.key_object != Some(object)
                || row.object != Some(object)
                || *count > 1;
            fact.ooc_deployed |= row.deployed.is_some();
        }
        records.push((
            &row.key,
            row.key_object,
            row.object,
            active,
            row.payload_digest,
            row.deployed.as_ref().map(|m| &m.mark),
        ));
    }
    let mut deployed = Vec::new();
    for object in objects {
        if let Some(marker) = store.deployed.get(object) {
            charge(&marker.mark.arena, &mut text)?;
            deployed.push((*object, &marker.mark));
        }
        let fact = facts.get_mut(object).ok_or(OwnerReadError::Integrity)?;
        fact.held_record |= fact.active_unit && !claims.contains_key(object);
        fact.held_record |= !fact.active_unit && claims.contains_key(object);
    }
    let facts: Vec<_> = facts.into_values().collect();
    let digest = mush_core::DefinitionDigest::for_canonical_value_with_limit(
        "tactics.movement.custody_preimage.v1",
        &json!({"schema":schema,"ic":ic,"facts":facts,"records":records,"deployed":deployed}),
        MAX_MOVEMENT_CUSTODY_BYTES,
    )
    .map_err(|_| OwnerReadError::Limit)?;
    Ok(MovementCustodyFacts { facts, digest })
}
impl OwnerReadService for MovementCustodyReader {
    type State = UnitStore;
    type Request = MovementCustodyRequest;
    type Output = MovementCustodyFacts;
    fn read(
        &self,
        state: &UnitStore,
        ctx: &OwnerReadContext<'_>,
        request: MovementCustodyRequest,
    ) -> Result<Self::Output, OwnerReadError> {
        if ctx.requester().as_str() != "sim" {
            return Err(OwnerReadError::Authority);
        }
        let result = derive(state, &request.objects)?;
        let digest = ctx.guard_state::<UnitStore, _>(move |store| {
            derive(store, &request.objects).map(|facts| facts.digest)
        })?;
        if digest != result.digest {
            return Err(OwnerReadError::Integrity);
        }
        Ok(result)
    }
}

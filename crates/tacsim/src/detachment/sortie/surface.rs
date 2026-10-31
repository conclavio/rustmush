//! Physical crew reach the existing surface host through their real field flight.
//! A selection is planning data; the host still fences K6 and unit permissions.
use super::runtime::error as bad;
use super::{
    facts,
    model::*,
    prepare,
    runtime::{Read, SortieHandles},
};
use crate::detachment::crew::CrewDuty;
use mush_core::{ObjId, ObjKind, World};
use mush_tactics::detachment::planning::PlannedController;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurfaceCrew {
    pub operation: OperationId,
    pub flight: FlightKey,
    pub operator: ObjId,
    /// Existing lease/contract principal, independently of the field leader.
    pub principal: ObjId,
}
pub(super) struct Selection {
    pub crew: SurfaceCrew,
    pub flight: Flight,
    pub lease: crate::port::tv::TvLease,
    pub observations: Value,
}
pub(super) fn selected(
    world: &World,
    read: &mut impl Read,
    h: &SortieHandles,
    actor: ObjId,
    unit: ObjId,
) -> Result<Selection, String> {
    if !h.enabled {
        return Err("Physical surface crew unavailable.".into());
    }
    let reservation = read
        .get(&h.reservations, &actor)?
        .ok_or("No physical field assignment.")?;
    reservation.validate()?;
    let id = OperationId {
        organizer: reservation.organizer,
        draft: reservation.draft,
        activation: reservation.revision,
    };
    let op = prepare::operation(read, h, id)?;
    if op.ended.is_some()
        || !matches!(op.roster.duty, CrewDuty::Mining | CrewDuty::Combat)
        || read.get(&h.active, &id.reference())?.as_ref() != Some(&op.roster)
    {
        // Doc 32 cut CF.6: an escort's combat contract drives its unit over the open ground
        // like a mining crew's; recovery stays its own duty's.
        return Err("No active mining or combat assignment.".into());
    }
    facts::reservation(&op.roster, actor, Some(reservation))?;
    facts::reservation(&op.roster, unit, read.get(&h.reservations, &unit)?)?;
    let member = op
        .roster
        .plan
        .slots
        .iter()
        .find(|s| s.unit == Some(unit) && s.controller.character() == Some(actor))
        .ok_or("Not this unit's accepted operator.")?;
    let (_, flight) = prepare::current(read, h, unit)?;
    prepare::membership(&op, &flight)?;
    if flight.operation != id
        || flight.operator != actor
        || !((flight.phase == Phase::Field && facts::on_site(world, &flight))
            || (flight.phase == Phase::Returned && facts::in_bay(world, &flight)))
    {
        return Err("The assigned unit is not physically on its field site.".into());
    }
    let npc = matches!(member.controller, PlannedController::Npc { .. });
    if !facts::live(
        world,
        actor,
        if npc { ObjKind::Npc } else { ObjKind::Player },
    ) {
        return Err("Field operator unavailable.".into());
    }
    let station = facts::station(world, actor, unit, npc)?;
    let lease = world
        .module_state::<crate::TacsimStore>()
        .and_then(|s| s.tv_lease_on(unit))
        .cloned()
        .ok_or("This surface host needs the rig's actual lease.")?;
    lease.check()?;
    if lease.tv != unit || lease.hull != flight.carrier || !lease.live() {
        return Err("The rig's lease or carrier changed.".into());
    }
    let contract = if npc {
        read.get(&h.contracts, &actor)?
    } else {
        None
    };
    if npc {
        let c = contract.as_ref().ok_or("NPC employment ended.")?;
        c.validate()?;
        if c.npc != actor
            || c.employer != id.organizer
            || lease.pilot != c.employer
            || !c.terms.duties.contains(&op.roster.duty)
            || op.roster.npc_terms.get(&actor) != Some(&c.terms_hash)
        {
            return Err(
                "The rig's lessee must employ this operator for the assignment's accepted duty."
                    .into(),
            );
        }
    } else if lease.pilot != actor {
        return Err("This surface host needs the player's own mining lease.".into());
    }
    let unit_state = mush_tactics::module::unit_of(world, unit).ok_or("Unit unavailable.")?;
    let condition =
        crate::field_ops::assess_tacsim_unit(unit_state, None).map_err(|e| e.to_string())?;
    if !condition.has_power || !condition.has_control {
        return Err("The unit lacks working power or controls.".into());
    }
    Ok(Selection {
        crew: SurfaceCrew {
            operation: id,
            flight: flight.key,
            operator: actor,
            principal: lease.pilot,
        },
        observations: json!({"flight":flight,"station":station,"contract":contract,"lease":lease,"condition":condition,
            "person":world.get(actor).map(|o|(o.kind,o.location))}),
        flight,
        lease,
    })
}

fn own_rig(
    world: &World,
    read: &mut impl Read,
    actor: ObjId,
) -> Option<crate::surface::act::RigAt> {
    let h = world.module_state::<SortieHandles>()?;
    let reservation = read.get(&h.reservations, &actor).ok()??;
    let id = OperationId {
        organizer: reservation.organizer,
        draft: reservation.draft,
        activation: reservation.revision,
    };
    let op = prepare::operation(read, h, id).ok()?;
    let unit = op
        .roster
        .plan
        .slots
        .iter()
        .find(|s| s.controller.character() == Some(actor))?
        .unit?;
    let selected = selected(world, read, h, actor, unit).ok()?;
    Some(crate::surface::act::RigAt {
        tv: unit,
        slot: selected.lease.slot,
        mark: selected.lease.mark.clone(),
        hull: selected.lease.hull,
        record: selected.lease,
        crew: Some(Box::new(selected.crew)),
        field_order: None,
    })
}
pub(crate) fn rig_in(
    engine: &mush_core::Engine,
    actor: ObjId,
) -> Option<crate::surface::act::RigAt> {
    own_rig(&engine.world, &mut &*engine, actor)
}
pub(crate) fn rig(ctx: &mush_core::Ctx<'_>) -> Option<crate::surface::act::RigAt> {
    own_rig(ctx.w, &mut &*ctx, ctx.actor)
}
pub(crate) fn unload_ready(
    ctx: &mush_core::Ctx<'_>,
    record: &crate::surface::excursion::RigRecord,
) -> bool {
    record.field_crew.as_deref().is_none_or(|crew| {
        own_rig(ctx.w, &mut &*ctx, crew.operator)
            .is_some_and(|rig| rig.crew.as_deref() == Some(crew) && matches!(crate::surface::act::where_is(ctx.w,record.tv),crate::surface::act::Where::Bay{hull} if hull==rig.hull))
            && working(ctx, record, false)
    })
}

fn body_live(world: &World, record: &crate::surface::excursion::RigRecord, moving: bool) -> bool {
    let Some(crew) = record.field_crew.as_deref() else {
        return true;
    };
    let npc = world
        .get(crew.operator)
        .is_some_and(|o| o.kind == ObjKind::Npc);
    facts::station(world, crew.operator, record.tv, npc).is_ok()
        && mush_tactics::module::unit_of(world, record.tv)
            .and_then(|u| crate::field_ops::assess_tacsim_unit(u, None).ok())
            .is_some_and(|c| c.has_power && c.has_control && (!moving || c.can_move))
}
pub(crate) fn working(
    ctx: &mush_core::Ctx<'_>,
    record: &crate::surface::excursion::RigRecord,
    moving: bool,
) -> bool {
    let Some(crew) = record.field_crew.as_deref() else {
        return true;
    };
    if !body_live(ctx.w, record, moving) {
        return false;
    }
    let Some(control) = ctx
        .w
        .module_state::<SortieHandles>()
        .and_then(|h| h.controls.as_ref())
    else {
        return false;
    };
    ctx.read_owner(
        control,
        vec![mush_sim::crew_control::CrewControlQuery {
            unit: record.tv,
            operator: crew.operator,
        }],
    )
    .is_ok_and(|v| v == [true])
}
pub(crate) fn working_preimage(
    ctx: &mut mush_core::PrepareCtx<'_>,
    txn: &mut mush_core::ActionTxn<'_>,
    record: &crate::surface::excursion::RigRecord,
    moving: bool,
) -> Result<bool, mush_core::ActionError> {
    let Some(crew) = record.field_crew.as_deref() else {
        return Ok(true);
    };
    let world = ctx.world().map_err(bad)?;
    let body = body_live(world, record, moving);
    let control = world
        .module_state::<SortieHandles>()
        .and_then(|h| h.controls.clone())
        .ok_or_else(|| bad("Crew controls unavailable."))?;
    let actual = ctx
        .read_owner_preimage(
            txn,
            &control,
            vec![mush_sim::crew_control::CrewControlQuery {
                unit: record.tv,
                operator: crew.operator,
            }],
        )
        .map_err(bad)?;
    Ok(body && actual == [true])
}

pub(crate) fn reconcile_return(
    ctx: &mush_core::PrepareCtx<'_>,
    txn: &mut mush_core::ActionTxn<'_>,
    record: &mut crate::surface::excursion::RigRecord,
) -> Result<(), String> {
    let Some(crew) = record.field_crew.as_deref().cloned() else {
        return Ok(());
    };
    if record.act.is_some() || record.run.is_some() {
        return Ok(());
    }
    let world = ctx.world().map_err(|e| e.to_string())?;
    let h = world
        .module_state::<SortieHandles>()
        .ok_or("Field history unavailable.")?;
    let flight = txn
        .get(&h.flights, &crew.flight)
        .map_err(|e| e.to_string())?
        .cloned()
        .ok_or("Field history unavailable.")?;
    if flight.operation != crew.operation || flight.operator != crew.operator {
        return Err("Field work binding changed.".into());
    }
    if flight.phase == Phase::Returned && facts::in_bay(world, &flight) {
        adopt_record(
            &crew,
            &flight,
            record,
            record.excursion.as_ref().map_or(1, |e| e.slot),
            None,
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

const KIND: &str = "detachment_surface_crew_v1";
const WORK_KIND: &str = "detachment_surface_work_v1";
pub(crate) fn work_key(
    actor: ObjId,
    unit: ObjId,
) -> Result<mush_core::RevisionKey, mush_core::ActionError> {
    let mut key = revision_key(actor, unit)?;
    key.kind = mush_core::RevisionKind::new(mush_core::CanonicalId::new(WORK_KIND).map_err(bad)?);
    Ok(key)
}
pub(crate) fn revision_key(
    actor: ObjId,
    unit: ObjId,
) -> Result<mush_core::RevisionKey, mush_core::ActionError> {
    use mush_core::{CanonicalId, OwnerId, RevisionKey, RevisionKind, SubjectKey};
    if actor.0 == 0 || unit.0 == 0 {
        return Err(bad("Invalid field surface coordinate."));
    }
    Ok(RevisionKey {
        owner: OwnerId::new(crate::NS).map_err(bad)?,
        kind: RevisionKind::new(CanonicalId::new(KIND).map_err(bad)?),
        subject: SubjectKey::Object { id: actor },
        facet: Some(CanonicalId::new(unit.0.to_string()).map_err(bad)?),
    })
}
struct Validator(std::sync::Arc<SortieHandles>);
impl mush_core::RevisionValidator for Validator {
    fn current_generation(
        &self,
        ctx: &mush_core::RevisionValidatorContext<'_>,
        key: &mush_core::RevisionKey,
    ) -> Result<Option<u64>, mush_core::ActionError> {
        let mush_core::SubjectKey::Object { id: actor } = key.subject else {
            return Ok(None);
        };
        let Some(unit) = key
            .facet
            .as_ref()
            .and_then(|s| s.as_str().parse::<u32>().ok())
            .map(ObjId)
        else {
            return Ok(None);
        };
        if work_key(actor, unit).ok().as_ref() == Some(key) {
            let w = ctx.world();
            let npc = w.get(actor).is_some_and(|o| o.kind == ObjKind::Npc);
            let facts = json!({"station":facts::station(w,actor,unit,npc),"unit":mush_tactics::module::unit_of(w,unit).map(|u|u.revision())});
            let hash = mush_core::CanonicalHash::for_canonical_value(
                "tacsim.surface_work.v1",
                1,
                &facts,
                16 * 1024,
            )
            .map_err(bad)?;
            let mut bytes = [0; 8];
            bytes.copy_from_slice(&hash.as_bytes()[..8]);
            return Ok(Some(u64::from_be_bytes(bytes).max(1)));
        }
        if revision_key(actor, unit).ok().as_ref() != Some(key) {
            return Ok(None);
        }
        let Ok(actual) = selected(ctx.world(), &mut &*ctx, &self.0, actor, unit) else {
            return Ok(None);
        };
        let hash = mush_core::CanonicalHash::for_canonical_value(
            "tacsim.surface_crew.v1",
            1,
            &actual.observations,
            16 * 1024,
        )
        .map_err(super::runtime::error)?;
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&hash.as_bytes()[..8]);
        Ok(Some(u64::from_be_bytes(bytes).max(1)))
    }
}
pub(crate) fn bridge_descriptor() -> Result<mush_core::DefinitionDescriptor, mush_core::ModError> {
    let invalid = |e| mush_core::ModError::msg(format!("{e}"));
    mush_core::DefinitionDescriptor::new(
        mush_core::DefinitionKind::new("bridge").map_err(invalid)?,
        mush_core::DefinitionId::new("tacsim_surface_act_unit_rights").map_err(invalid)?,
        1,
        json!({"format":1,"purpose":"current_physical_surface_operator_rights"}),
    )
    .map_err(|e| mush_core::ModError::msg(e.to_string()))
}
pub(super) fn register(
    reg: &mut mush_core::Registrar<'_>,
    h: std::sync::Arc<SortieHandles>,
    action: Option<&mush_core::DefinitionRef>,
) -> Result<(), mush_core::ModError> {
    if h.enabled
        && let Some(action) = action
    {
        let invalid = |e| mush_core::ModError::msg(format!("{e}"));
        let bridge = reg.definition(bridge_descriptor()?)?;
        reg.interaction_bridge(mush_core::BridgeDescriptor::new(
            bridge,
            mush_core::OwnerId::new(crate::NS).map_err(invalid)?,
            mush_core::OwnerId::new("org").map_err(invalid)?,
            action.clone(),
        ))?;
    }
    reg.revision_validator(
        mush_core::RevisionKind::new(
            mush_core::CanonicalId::new(KIND)
                .map_err(|e| mush_core::ModError::msg(e.to_string()))?,
        ),
        Validator(h.clone()),
    )?;
    reg.revision_validator(
        mush_core::RevisionKind::new(
            mush_core::CanonicalId::new(WORK_KIND)
                .map_err(|e| mush_core::ModError::msg(e.to_string()))?,
        ),
        Validator(h),
    )
}
pub(crate) fn admit(
    world: &World,
    txn: &mut mush_core::ActionTxn<'_>,
    actor: ObjId,
    unit: ObjId,
    expected: &SurfaceCrew,
    moving: bool,
) -> Result<Flight, mush_core::ActionError> {
    let h = world
        .module_state::<SortieHandles>()
        .ok_or_else(|| bad("Physical crew unavailable."))?;
    let selected = selected(world, &mut &mut *txn, h, actor, unit).map_err(bad)?;
    if selected.crew != *expected {
        return Err(bad("Physical crew or flight changed."));
    }
    if moving
        && !mush_tactics::module::unit_of(world, unit)
            .and_then(|u| crate::field_ops::assess_tacsim_unit(u, None).ok())
            .is_some_and(|c| c.can_move)
    {
        return Err(bad("The unit cannot move."));
    }
    Ok(selected.flight)
}

pub(crate) fn adopt_record(
    crew: &SurfaceCrew,
    flight: &Flight,
    record: &mut crate::surface::excursion::RigRecord,
    slot: u8,
    battery: Option<&crate::definitions::tools::BatteryDef>,
) -> Result<(), mush_core::ActionError> {
    use crate::surface::excursion::{Charge, Excursion, ExcursionId, charge_at};
    if record.field_crew.as_deref() != Some(crew) {
        record.field_work = None;
    }
    if record.sortie.is_some() {
        return Err(bad("The DT flight already owns this hull's epoch."));
    }
    if flight.phase == Phase::Returned {
        if record.act.is_some() || record.run.is_some() {
            return Err(bad("Earlier surface work is still settling."));
        }
        if record.field_crew.as_deref().is_some_and(|old| old != crew) && record.excursion.is_some()
        {
            return Err(bad("Another surface excursion is still open."));
        }
        let at = flight
            .closed
            .as_ref()
            .ok_or_else(|| bad("Field return is not confirmed."))?
            .at
            .at;
        record.excursion = None;
        if !record.charge.in_bay {
            record.charge = Charge {
                value_cu: record.charge.value_cu,
                at_ic: record.charge.at_ic.max(at),
                in_bay: true,
            };
        }
        record.field_crew = Some(Box::new(crew.clone()));
        return record.check().map_err(bad);
    }
    let excursion = Excursion {
        id: ExcursionId {
            tv: flight.key.unit,
            launch_ic: flight.requested.at,
        },
        operator: crew.operator,
        hull: flight.carrier,
        slot,
        site: flight.site.clone(),
    };
    if record.excursion.as_ref().is_some_and(|e| e != &excursion)
        || record.field_crew.as_deref().is_some_and(|old| old != crew) && record.excursion.is_some()
    {
        return Err(bad("Another surface excursion is still open."));
    }
    if record.charge.in_bay {
        if record.charge.at_ic > flight.requested.at {
            return Err(bad("Bay charge is newer than the actual field departure."));
        }
        let value = battery.map_or(record.charge.value_cu, |b| {
            charge_at(&record.charge, flight.requested.at, b)
        });
        record.charge = Charge {
            value_cu: value,
            at_ic: flight.due.at,
            in_bay: false,
        };
    }
    record.excursion = Some(excursion);
    record.field_crew = Some(Box::new(crew.clone()));
    record.check().map_err(bad)
}

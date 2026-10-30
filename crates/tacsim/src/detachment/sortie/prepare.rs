use super::{
    facts,
    model::*,
    runtime::{self, Command, Intent, Read, SortieHandles},
};
use mush_core::{CanonicalHash, ClockStamp, ObjId, ObjKind, World};
use mush_org::unit_authority::{UnitCapability, operation::revision_key};
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeSet;

#[derive(Serialize)]
pub(super) struct Write<T> {
    pub before: Option<T>,
    pub after: T,
}
#[derive(Serialize)]
pub(super) struct Snapshot {
    pub hash: CanonicalHash,
    operation: OperationId,
    record: Option<Box<Write<OperationRecord>>>,
    index: Option<Write<FlightIndex>>,
    heads: Vec<Write<UnitHead>>,
    flights: Vec<Write<Flight>>,
    observations: Vec<Value>,
    lines: Vec<(ObjId, crate::surface::black_box::BlackBoxLine)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transport: Option<Box<mush_propagation::field_transport::Request>>,
}
impl Snapshot {
    pub(super) fn new(operation: OperationId) -> Result<Self, String> {
        Ok(Self {
            hash: CanonicalHash::for_canonical_value("tacsim.flight.empty.v1", 1, &json!({}), 1024)
                .map_err(|e| e.to_string())?,
            operation,
            record: None,
            index: None,
            heads: Vec::new(),
            flights: Vec::new(),
            observations: Vec::new(),
            lines: Vec::new(),
            transport: None,
        })
    }
    pub fn generation(&self) -> u64 {
        let mut bytes = [0; 8];
        bytes.copy_from_slice(&self.hash.as_bytes()[..8]);
        u64::from_be_bytes(bytes).max(1)
    }
    pub fn apply(
        mut self,
        txn: &mut mush_core::ActionTxn<'_>,
        ctx: &mut mush_core::PrepareCtx<'_>,
        handles: &SortieHandles,
        black_box: Option<&runtime::BlackBoxHandle>,
    ) -> Result<Value, mush_core::ActionError> {
        // Log failure never invents a missing physical departure. S.1's
        // unrecorded flag survives in the authoritative flight history.
        if let Some(request) = self.transport.take() {
            let queued = ctx
                .prepare_owner(
                    handles
                        .transport
                        .as_ref()
                        .ok_or_else(|| runtime::error("Field transport unavailable."))?,
                    *request,
                )
                .map_err(runtime::error)?;
            ctx.require_owner_preparation_binding(txn, queued.binding())
                .map_err(runtime::error)?;
            let write = self
                .record
                .as_mut()
                .ok_or_else(|| runtime::error("Field source history unavailable."))?;
            let roster = mush_tactics::detachment::roster::DetachmentRoster::restore(
                write
                    .after
                    .command
                    .as_deref()
                    .ok_or_else(|| runtime::error("Field command unavailable."))?,
            )
            .map_err(runtime::error)?;
            write
                .after
                .orders
                .as_mut()
                .ok_or_else(|| runtime::error("Field source history unavailable."))?
                .attach(write.after.id, &roster, queued.envelopes())
                .map_err(runtime::error)?;
            write.after.validate().map_err(runtime::error)?;
        }
        let mut flights = self.flights;
        for (unit, line) in self.lines {
            let appended = crate::surface::black_box::append(ctx, black_box, unit, line)?;
            if appended
                && let Some(write) = flights.iter_mut().find(|w| w.after.key.unit == unit)
                && write.after.phase == Phase::Field
                && let Some(epoch) = write.after.epoch.as_mut()
            {
                epoch.unrecorded = false;
            }
        }
        if let Some(write) = self.record {
            save(
                txn,
                &handles.operations,
                &write.after.id,
                write.before.is_some(),
                write.after.clone(),
            )?;
        }
        if let Some(write) = self.index {
            save(
                txn,
                &handles.index,
                &(),
                write.before.is_some(),
                write.after,
            )?;
        }
        for write in self.heads {
            save(
                txn,
                &handles.heads,
                &write.after.flight.unit,
                write.before.is_some(),
                write.after.clone(),
            )?;
        }
        let mut result = Vec::new();
        for write in flights {
            write.after.validate().map_err(runtime::error)?;
            result.push(json!({"unit":write.after.key.unit,"flight":write.after.key.ordinal,"phase":write.after.phase}));
            save(
                txn,
                &handles.flights,
                &write.after.key,
                write.before.is_some(),
                write.after.clone(),
            )?;
        }
        Ok(json!({"operation":self.operation,"flights":result}))
    }
}
fn save<K, V>(
    txn: &mut mush_core::ActionTxn<'_>,
    handle: &mush_core::StateFamilyHandle<K, V>,
    key: &K,
    exists: bool,
    value: V,
) -> Result<(), mush_core::ActionError>
where
    K: Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    if exists {
        txn.replace(handle, key, value)
    } else {
        txn.insert(handle, key, value)
    }
    .map_err(runtime::error)
}
pub(super) fn operation(
    read: &mut impl Read,
    h: &SortieHandles,
    id: OperationId,
) -> Result<OperationRecord, String> {
    let row = read
        .get(&h.operations, &id)?
        .ok_or("Operation unavailable.")?;
    row.validate()?;
    if row.id != id {
        return Err("Operation identity mismatch.".into());
    }
    Ok(row)
}
pub(super) fn current(
    read: &mut impl Read,
    h: &SortieHandles,
    unit: ObjId,
) -> Result<(UnitHead, Flight), String> {
    let head = read.get(&h.heads, &unit)?.ok_or("No unit flight.")?;
    head.validate()?;
    if head.flight.unit != unit {
        return Err("Flight head identity mismatch.".into());
    }
    let flight = read
        .get(&h.flights, &head.flight)?
        .ok_or("Flight history unavailable.")?;
    flight.validate()?;
    if flight.key != head.flight {
        return Err("Flight identity mismatch.".into());
    }
    Ok((head, flight))
}
fn member(op: &OperationRecord, slot: u16) -> Result<(ObjId, ObjId), String> {
    let row = op
        .roster
        .plan
        .slots
        .iter()
        .find(|s| s.slot.get() == slot)
        .ok_or("Unknown operation slot.")?;
    Ok((
        row.unit.ok_or("Unit unavailable.")?,
        row.controller.character().ok_or("Operator unavailable.")?,
    ))
}
fn selected(
    read: &mut impl Read,
    h: &SortieHandles,
    id: OperationId,
    slot: u16,
) -> Result<(OperationRecord, Flight), String> {
    let op = operation(read, h, id)?;
    let (unit, operator) = member(&op, slot)?;
    let (_, flight) = current(read, h, unit)?;
    membership(&op, &flight)?;
    if flight.operation != id || flight.slot != slot || flight.operator != operator {
        return Err("Slot's current flight changed.".into());
    }
    Ok((op, flight))
}
pub(super) fn membership(op: &OperationRecord, flight: &Flight) -> Result<(), String> {
    if op.id != flight.operation
        || !op.flights.contains(&flight.key)
        || member(op, flight.slot)? != (flight.key.unit, flight.operator)
        || flight.carrier != op.roster.plan.carrier
    {
        return Err("Flight does not match its accepted operation.".into());
    }
    Ok(())
}
fn due(from: ClockStamp, seconds: u32) -> Result<ClockStamp, String> {
    Ok(ClockStamp {
        at: from
            .at
            .checked_add(i64::from(seconds))
            .ok_or("Deployment clock exhausted.")?,
        ..from
    })
}
pub(super) fn snapshot(
    world: &World,
    read: &mut impl Read,
    h: &SortieHandles,
    actor: ObjId,
    intent: &Intent,
) -> Result<Snapshot, String> {
    if !h.enabled
        || runtime::pending(world, actor) != Some(intent)
        || !world
            .now()
            .checked_sub(intent.utc)
            .is_some_and(|age| (0..=300).contains(&age))
        || world.world_ic_now().clock != intent.ic.clock
        || world
            .module_state::<crate::TacsimStore>()
            .is_none_or(|s| !s.enabled())
    {
        return Err("Deployment request expired or unavailable.".into());
    }
    let before_index = read.get(&h.index, &())?;
    if world.world_timeline().is_frozen()
        && matches!(
            intent.command,
            Command::Launch { .. } | Command::Recover { .. }
        )
    {
        return Err("World time is frozen; deployment waits for unfreeze.".into());
    }
    let mut index = before_index.clone().unwrap_or_default();
    index.validate()?;
    if before_index.is_none() && !matches!(intent.command, Command::Launch { .. }) {
        return Err("Outstanding flight index unavailable.".into());
    }
    let mut patch = match &intent.command {
        Command::Launch { operation, slots } => {
            launch(world, read, h, actor, intent, *operation, slots)?
        }
        Command::Recover {
            operation,
            slot,
            unit,
        } => {
            let (op, flight) = selected(read, h, *operation, *slot)?;
            if flight.key.unit != *unit
                || (actor != operation.organizer && actor != flight.operator)
                || flight.phase != Phase::Field
                || !facts::at_ramp(world, &flight)
                || !(crate::detachment::planning::projection::physically_aboard(
                    world,
                    actor,
                    flight.carrier,
                ) || crate::detachment::planning::projection::physically_aboard(
                    world,
                    actor,
                    flight.key.unit,
                ))
            {
                return Err(
                    "Return unavailable. Bring the unit physically to the carrier's ramp.".into(),
                );
            }
            let berth = facts::berth(world, flight.carrier)?;
            if berth.domain != flight.domain
                || berth.bay_room != flight.bay_room
                || berth.bay_slot != flight.bay_slot
            {
                return Err("Carrier's recovery berth changed.".into());
            }
            let mut next = flight.clone();
            let recovery = facts::recover_unit(world, &op.roster, &flight)?;
            next.phase = Phase::Returning;
            next.returner = Some(actor);
            next.revision = next
                .revision
                .checked_add(1)
                .ok_or("Flight revision exhausted.")?;
            next.due = due(intent.ic.clone(), crate::surface::excursion::RECOVER_S)?;
            let mut patch = Snapshot::new(*operation)?;
            patch.observations.push(json!({"recovery":recovery,"parent":world.module_state::<mush_sim::SpatialState>().map(|s|s.parent_of(*unit))}));
            patch.flights.push(Write {
                before: Some(flight),
                after: next,
            });
            patch
        }
        Command::Cancel { operation, slot } => {
            let (_, flight) = selected(read, h, *operation, *slot)?;
            if actor != operation.organizer
                || flight.phase != Phase::Departing
                || !facts::in_bay(world, &flight)
            {
                return Err("Only an unlaunched member can be cancelled.".into());
            }
            let mut next = flight.clone();
            next.phase = Phase::Cancelled;
            next.revision = next
                .revision
                .checked_add(1)
                .ok_or("Flight revision exhausted.")?;
            let mut patch = Snapshot::new(*operation)?;
            patch.flights.push(Write {
                before: Some(flight),
                after: next,
            });
            patch
        }
        Command::End { operation: id } => {
            let old = operation(read, h, *id)?;
            if actor != id.organizer || old.ended.is_some() {
                return Err("Operation unavailable.".into());
            }
            let mut next = old.clone();
            next.ended = Some(intent.ic.clone());
            let mut patch = Snapshot::new(*id)?;
            // End cancels only departures still physically aboard. Field epochs
            // remain open and recoverable; a disappeared member is unresolved.
            for row in &old.roster.plan.slots {
                let unit = row.unit.ok_or("Unit unavailable.")?;
                if read.get(&h.heads, &unit)?.is_some() {
                    let (_, flight) = current(read, h, unit)?;
                    if flight.operation == *id
                        && flight.phase == Phase::Departing
                        && facts::in_bay(world, &flight)
                    {
                        let mut next = flight.clone();
                        next.phase = Phase::Cancelled;
                        next.revision = next
                            .revision
                            .checked_add(1)
                            .ok_or("Flight revision exhausted.")?;
                        patch.flights.push(Write {
                            before: Some(flight),
                            after: next,
                        });
                    }
                }
            }
            patch.record = Some(Box::new(Write {
                before: Some(old),
                after: next,
            }));
            patch
        }
        Command::Finish {
            flight: key,
            revision,
        } => finish(world, read, h, actor, intent, *key, *revision)?,
        Command::Refresh {
            operation: id,
            facts,
        } => {
            let old = operation(read, h, *id)?;
            if actor != id.organizer
                && !old
                    .roster
                    .plan
                    .slots
                    .iter()
                    .any(|s| s.controller.character() == Some(actor))
            {
                return Err("Operation unavailable.".into());
            }
            let mut next = old.clone();
            let (roster, observations) = super::field::reconcile(world, read, h, &old, facts)?;
            next.command = Some(Box::new(roster.checkpoint()));
            let mut patch = Snapshot::new(*id)?;
            patch.observations = observations;
            patch.record = Some(Box::new(Write {
                before: Some(old),
                after: next,
            }));
            patch
        }
        Command::Message(request) => message(world, read, h, actor, request)?,
    };
    for write in &patch.flights {
        if write.after.phase.open() {
            index.units.insert(write.after.key.unit);
        } else {
            index.units.remove(&write.after.key.unit);
        }
    }
    index.validate()?;
    if before_index.as_ref() != Some(&index) {
        patch.index = Some(Write {
            before: before_index,
            after: index,
        });
    }
    for flight in &patch.flights {
        flight.after.validate()?
    }
    if let Some(op) = &patch.record {
        op.after.validate()?
    }
    patch.hash = CanonicalHash::for_canonical_value(
        "tacsim.detachment_flight_facts.v1",
        1,
        &json!({"actor":actor,"intent":intent,"patch":patch,"clock":world.world_ic_now().clock}),
        256 * 1024,
    )
    .map_err(|e| e.to_string())?;
    Ok(patch)
}
fn launch(
    world: &World,
    read: &mut impl Read,
    h: &SortieHandles,
    actor: ObjId,
    intent: &Intent,
    id: OperationId,
    slots: &[runtime::Assignment],
) -> Result<Snapshot, String> {
    let roster = read
        .get(&h.active, &id.reference())?
        .ok_or("Accepted roster unavailable.")?;
    roster.validate()?;
    if actor != id.organizer
        || OperationId::of(&roster) != id
        || !roster.active()
        || !facts::live(world, actor, ObjKind::Player)
        || !crate::detachment::planning::projection::physically_aboard(
            world,
            actor,
            roster.plan.carrier,
        )
        || slots.is_empty()
        || slots.len() > 6
        || slots.windows(2).any(|p| p[0].slot >= p[1].slot)
    {
        return Err("Accepted launch selection changed or is unavailable.".into());
    }
    let old_op = read.get(&h.operations, &id)?;
    if let Some(old) = &old_op {
        old.validate()?;
        if old.id != id || old.roster != roster || old.ended.is_some() {
            return Err("Operation already ended or changed.".into());
        }
    }
    let mut patch = Snapshot::new(id)?;
    if old_op.is_none() {
        patch.record = Some(Box::new(Write {
            before: None,
            after: OperationRecord {
                schema: 1,
                id,
                roster: roster.clone(),
                began: intent.ic.clone(),
                ended: None,
                flights: Vec::new(),
                command: None,
                orders: None,
            },
        }))
    }
    let berth = facts::berth(world, roster.plan.carrier)?;
    let ship = read.get(
        &h.ports.epochs,
        &crate::port::epoch::EpochKey(roster.plan.carrier),
    )?;
    for chosen in slots {
        let row = roster
            .plan
            .slots
            .iter()
            .find(|s| s.slot.get() == chosen.slot)
            .ok_or("Unknown launch slot.")?;
        if row.unit != Some(chosen.unit) || row.controller.character() != Some(chosen.operator) {
            return Err("Accepted assignment changed.".into());
        }
        facts::reservation(
            &roster,
            chosen.unit,
            read.get(&h.reservations, &chosen.unit)?,
        )?;
        facts::reservation(
            &roster,
            chosen.operator,
            read.get(&h.reservations, &chosen.operator)?,
        )?;
        let contract = read.get(&h.contracts, &chosen.operator)?;
        let person = facts::launch_person(world, &roster, chosen.slot, contract)?;
        let unit = facts::launch_unit(world, chosen.unit, roster.plan.carrier)?;
        if let Some(rig) = read.get(
            &h.ports.excursions,
            &crate::surface::excursion::RigKey(chosen.unit),
        )? && (rig.sortie.is_some()
            || rig.act.is_some()
            || rig.run.is_some()
            || rig.excursion.is_some())
        {
            return Err("Unit already belongs to a surface excursion.".into());
        }
        let old_head = read.get(&h.heads, &chosen.unit)?;
        if old_head.is_some() {
            let (_, last) = current(read, h, chosen.unit)?;
            if last.phase == Phase::Departing && last.operation == id {
                if last.site != berth.entry || last.domain != berth.domain {
                    return Err(
                        "Cancel the old site's pending departure before launching here.".into(),
                    );
                }
                let mut next = last.clone();
                next.revision = next
                    .revision
                    .checked_add(1)
                    .ok_or("Flight revision exhausted.")?;
                patch.flights.push(Write {
                    before: Some(last),
                    after: next,
                });
                patch
                    .observations
                    .push(json!({"unit":unit,"person":person,"roster":roster}));
                continue;
            }
            if last.phase.open() {
                return Err("Unit's previous flight is still open.".into());
            }
        }
        let ordinal = old_head
            .as_ref()
            .map_or(0, |h| h.flight.ordinal)
            .checked_add(1)
            .ok_or("Flight identity exhausted.")?;
        let key = FlightKey {
            unit: chosen.unit,
            ordinal,
        };
        if read.get(&h.flights, &key)?.is_some() {
            return Err("Flight identity is already present.".into());
        }
        let flight = Flight {
            schema: 1,
            key,
            operation: id,
            revision: 1,
            slot: chosen.slot,
            launcher: actor,
            operator: chosen.operator,
            carrier: roster.plan.carrier,
            site: berth.entry.clone(),
            domain: berth.domain,
            ramp: berth.ramp.clone(),
            bay_room: berth.bay_room,
            bay_slot: berth.bay_slot.clone(),
            requested: intent.ic.clone(),
            due: due(intent.ic.clone(), crate::surface::excursion::LAUNCH_S)?,
            phase: Phase::Departing,
            epoch: None,
            carrier_epoch: ship.as_ref().and_then(|s| s.open.as_ref()).map(|o| o.id),
            closed: None,
            returner: None,
        };
        patch.heads.push(Write {
            before: old_head,
            after: UnitHead {
                schema: 1,
                flight: key,
            },
        });
        patch.flights.push(Write {
            before: None,
            after: flight,
        });
        if patch.record.is_none() {
            patch.record = Some(Box::new(Write {
                before: old_op.clone(),
                after: old_op.clone().ok_or("Operation disappeared.")?,
            }));
        }
        patch
            .record
            .as_mut()
            .ok_or("Operation unavailable.")?
            .after
            .flights
            .push(key);
        patch
            .observations
            .push(json!({"unit":unit,"person":person,"ship":ship,"roster":roster}));
    }
    Ok(patch)
}

fn finish(
    world: &World,
    read: &mut impl Read,
    h: &SortieHandles,
    actor: ObjId,
    intent: &Intent,
    key: FlightKey,
    revision: u64,
) -> Result<Snapshot, String> {
    let (_, flight) = current(read, h, key.unit)?;
    if flight.key != key
        || flight.revision != revision
        || actor != flight.continuation_actor()
        || !known_clock(world, &flight.requested)
        || !known_clock(world, &flight.due)
    {
        return Err("Flight completion changed.".into());
    }
    if matches!(flight.phase, Phase::Departing | Phase::Returning)
        && world.world_ic_now().at < flight.due.at
    {
        return Err("Deployment step is not due.".into());
    }
    let op = operation(read, h, flight.operation)?;
    membership(&op, &flight)?;
    let mut patch = Snapshot::new(flight.operation)?;
    let mut next = flight.clone();
    next.revision = next
        .revision
        .checked_add(1)
        .ok_or("Flight revision exhausted.")?;
    if flight.phase == Phase::Departing && facts::on_site(world, &flight) {
        let facts = epoch_facts(world, read, h, &flight)?;
        let excursion = crate::surface::excursion::Excursion {
            id: crate::surface::excursion::ExcursionId {
                tv: key.unit,
                launch_ic: flight.requested.at,
            },
            operator: flight.operator,
            hull: flight.carrier,
            slot: u8::try_from(flight.slot).map_err(|_| "Invalid epoch slot.")?,
            site: flight.site.clone(),
        };
        let (mut open, line) = crate::surface::black_box::open_epoch(
            key.unit,
            &excursion,
            flight.due.at,
            Some(&facts.0),
        );
        open.unrecorded = true;
        if facts.1 {
            patch.lines.push((
                key.unit,
                line.with_compartment(flight.bay_room, flight.bay_slot.clone())
                    .with_detachment(link(&op, &flight)),
            ))
        }
        next.epoch = Some(open);
        next.phase = Phase::Field;
    } else if matches!(flight.phase, Phase::Field | Phase::Returning)
        && facts::in_bay(world, &flight)
    {
        let open = flight.epoch.as_ref().ok_or("Open epoch unavailable.")?;
        let at = if flight.phase == Phase::Returning {
            flight.due.clone()
        } else {
            intent.ic.clone()
        };
        let kind = if flight.phase == Phase::Returning {
            mush_roll::commerce::ic_claims::IcCloseKind::ReturnedToBase {
                base: flight.carrier,
            }
        } else {
            mush_roll::commerce::ic_claims::IcCloseKind::RecoveredToBase {
                base: flight.carrier,
            }
        };
        next.closed = Some(FlightClose {
            at: at.clone(),
            kind,
            operator_aboard: crate::detachment::planning::projection::physically_aboard(
                world,
                flight.operator,
                flight.carrier,
            ),
            ordered_by: flight.returner,
        });
        next.phase = Phase::Returned;
        patch.lines.push((
            key.unit,
            crate::surface::black_box::BlackBoxLine::sortie_closed(key.unit, open, at.at, kind)
                .with_detachment(link(&op, &flight)),
        ));
    } else {
        return Err("Physical departure or return is not confirmed.".into());
    }
    patch.observations.push(json!({"parent":world.module_state::<mush_sim::SpatialState>().map(|s|s.parent_of(key.unit)),
        "unit_revision":mush_tactics::module::unit_of(world,key.unit).map(|u|u.revision())}));
    patch.flights.push(Write {
        before: Some(flight),
        after: next,
    });
    Ok(patch)
}
fn link(op: &OperationRecord, flight: &Flight) -> crate::surface::black_box::DetachmentLink {
    crate::surface::black_box::DetachmentLink {
        operation: op.id,
        detachment: op.roster.detachment,
        roster_revision: op.roster.revision,
        source_revision: op.roster.source_revision,
        slot: flight.slot,
        launcher: flight.launcher,
        operator: flight.operator,
    }
}
fn epoch_facts(
    world: &World,
    read: &mut impl Read,
    h: &SortieHandles,
    flight: &Flight,
) -> Result<(crate::surface::black_box::EpochFacts, bool), String> {
    let store = world
        .module_state::<crate::TacsimStore>()
        .ok_or("Tacsim unavailable.")?;
    let unit = mush_tactics::module::unit_of(world, flight.key.unit);
    let seal = unit
        .and_then(|unit| {
            crate::match_::seal::seal_rom_for(
                flight.key.unit,
                unit,
                &[],
                crate::arena::mind::SortieId::ic(flight.key.unit, flight.requested.at),
                &crate::match_::seal::IcSealConfig {
                    minds: &store.cfg.minds,
                    reflex: &store.cfg.arena.reflex,
                    principal: Some(crate::deliberation::order::CharacterRef::Character {
                        id: flight.operator,
                    }),
                    impounded: crate::match_::seal::core_impounded(world, unit),
                },
            )
            .ok()
        })
        .map(|seal| {
            crate::surface::black_box::SealFacts::of(
                &seal,
                crate::minds::licence_state::SealLicenceState::Doctrine,
            )
        });
    // Only outstanding flights are indexed; archived histories never make the
    // census or the driver grow without bound. An unreadable unrelated record
    // can suppress the optional log line, never the confirmed physical fact.
    let population = population(world, read, h, flight);
    let bounded = population.is_ok();
    let count = population.unwrap_or(0);
    Ok((
        crate::surface::black_box::EpochFacts {
            seal,
            class: unit.map(crate::definitions::unit::UnitClass::read),
            site_epochs: count,
            carrier: flight.carrier_epoch,
        },
        bounded,
    ))
}
fn population(
    world: &World,
    read: &mut impl Read,
    h: &SortieHandles,
    flight: &Flight,
) -> Result<u32, String> {
    let index = read
        .get(&h.index, &())?
        .ok_or("Outstanding flight index unavailable.")?;
    index.validate()?;
    let mut count = 0u32;
    for unit in index.units {
        if unit == flight.key.unit {
            continue;
        }
        let (_, other) = current(read, h, unit)?;
        if !other.phase.open() {
            return Err("Outstanding flight index changed.".into());
        }
        if other.epoch.is_some() && other.closed.is_none() && other.site == flight.site {
            count = count.saturating_add(1)
        }
    }
    let rigs = crate::surface::act::world_rigs(world);
    if rigs.len() > crate::surface::excursion::MAX_EXCURSIONS_WORLD as usize {
        return Err("Surface population exceeds census budget.".into());
    }
    for unit in rigs {
        if unit == flight.key.unit {
            continue;
        }
        if let Some(rig) = read.get(
            &h.ports.excursions,
            &crate::surface::excursion::RigKey(unit),
        )? && rig
            .sortie
            .as_ref()
            .is_some_and(|open| open.site == flight.site)
        {
            count = count.saturating_add(1)
        }
    }
    Ok(count)
}
pub(super) fn authority_keys(
    intent: &Intent,
    actor: ObjId,
) -> Result<Vec<mush_core::RevisionKey>, mush_core::ActionError> {
    let mut keys = BTreeSet::new();
    match &intent.command {
        Command::Launch { slots, .. } => {
            for slot in slots {
                keys.insert(revision_key(actor, slot.unit, UnitCapability::Deploy)?);
                keys.insert(revision_key(
                    slot.operator,
                    slot.unit,
                    UnitCapability::OperateCrew,
                )?);
            }
        }
        Command::Recover { unit, .. } => {
            keys.insert(revision_key(actor, *unit, UnitCapability::RecoverLoad)?);
        }
        Command::Refresh { facts, .. } => {
            for row in &facts.crew {
                if row.operate {
                    keys.insert(revision_key(
                        row.operator,
                        row.unit,
                        UnitCapability::OperateCrew,
                    )?);
                }
                if row.command_authorized {
                    keys.insert(command_key(row.operator, row.unit, row.command_role)?);
                }
            }
        }
        Command::Message(request) => {
            for row in &request.facts.crew {
                if row.operate {
                    keys.insert(revision_key(
                        row.operator,
                        row.unit,
                        UnitCapability::OperateCrew,
                    )?);
                }
                if row.command_authorized {
                    keys.insert(command_key(row.operator, row.unit, row.command_role)?);
                }
            }
            for right in &request.rights {
                if right.allowed {
                    keys.insert(command_key(right.issuer, right.unit, right.role_grant)?);
                }
            }
        }
        _ => {}
    }
    Ok(keys.into_iter().collect())
}
fn command_key(
    actor: ObjId,
    unit: ObjId,
    role: Option<mush_org::unit_authority::UnitGrantId>,
) -> Result<mush_core::RevisionKey, mush_core::ActionError> {
    match role {
        Some(grant) => {
            mush_org::unit_authority::operation::mission_revision_key(actor, unit, grant)
        }
        None => revision_key(actor, unit, UnitCapability::CommandAssignedAi),
    }
}

fn message(
    world: &World,
    read: &mut impl Read,
    h: &SortieHandles,
    actor: ObjId,
    request: &super::field_orders::Request,
) -> Result<Snapshot, String> {
    use super::field_orders::{self, Act, FieldOrders};
    use mush_propagation::{
        EnvelopeState,
        dispatch::DispatchRef,
        field_transport::{Message, Request as Transport},
        record::DispatchKind,
    };
    use mush_tactics::detachment::{Directive, OrderBook, OrderRequest};
    let old = operation(read, h, request.operation)?;
    if actor != old.id.organizer
        && !old
            .roster
            .plan
            .slots
            .iter()
            .any(|s| s.controller.character() == Some(actor))
    {
        return Err("Operation unavailable.".into());
    }
    let coords = field_orders::coordinates(&old, &request.act, actor)?;
    if coords.len() != request.rights.len()
        || coords
            .iter()
            .zip(&request.rights)
            .any(|((unit, issuer), r)| *unit != r.unit || *issuer != r.issuer)
    {
        return Err("Command targets changed.".into());
    }
    let (roster, observations) = super::field::reconcile(world, read, h, &old, &request.facts)?;
    let now = u64::try_from(world.world_ic_now().at)
        .map_err(|_| "Field clock unavailable.")?
        .checked_mul(1000)
        .ok_or("Field clock exhausted.")?;
    let mut orders = old
        .orders
        .as_deref()
        .cloned()
        .unwrap_or_else(|| FieldOrders::new(&roster));
    let mut book = OrderBook::restore(&orders.book, &roster).map_err(|e| e.to_string())?;
    let mut patch = Snapshot::new(old.id)?;
    patch.observations = observations;
    let allowed = |issuer, unit| {
        request
            .rights
            .iter()
            .any(|r| r.issuer == issuer && r.unit == unit && r.allowed)
    };
    match &request.act {
        Act::Issue {
            revision,
            term,
            target,
            directive,
            expires_at_ms,
        } => {
            if let Directive::Objective(payload) = directive {
                payload.validate(&roster)?;
            }
            let issued = book
                .issue(
                    &roster,
                    now,
                    OrderRequest {
                        issuer: actor,
                        roster_revision: *revision,
                        command_term: *term,
                        target: *target,
                        directive: directive.clone(),
                        expires_at_ms: *expires_at_ms,
                    },
                    |m| allowed(actor, m.unit) && roster.member_available(m.slot),
                )
                .map_err(|e| e.to_string())?;
            let mut messages = Vec::new();
            for slot in issued.recipients {
                let order = book.pending(slot).ok_or("Admitted message unavailable.")?;
                let bytes =
                    u32::try_from(serde_json::to_vec(order).map_err(|e| e.to_string())?.len())
                        .map_err(|_| "Field message too large.")?;
                messages.push(Message {
                    recipient: order
                        .recipient
                        .controller
                        .character()
                        .ok_or("Recipient unavailable.")?,
                    reference: DispatchRef {
                        version: 1,
                        key: field_orders::message_key(old.id, order.sequence, slot),
                        bytes,
                        origin: None,
                        fallback: None,
                    },
                });
                orders.outbound.remove(&slot);
            }
            patch.transport = Some(Box::new(Transport::Send(messages)));
        }
        Act::Poll => {
            for member in roster.members() {
                let Some(pending) = book.pending(member.slot).cloned() else {
                    continue;
                };
                let obsolete = pending.command_term != roster.term()
                    || pending.roster_revision != roster.revision()
                    || roster
                        .leader()
                        .is_none_or(|l| l.character != pending.issuer)
                    || pending.expires_at_ms.is_some_and(|end| now >= end);
                if obsolete {
                    book.discard_pending(member.slot, pending.sequence, now)
                        .map_err(|e| e.to_string())?;
                    orders.outbound.remove(&member.slot);
                    continue;
                }
                let envelope = orders.outbound.get(&member.slot).and_then(|id| {
                    world
                        .module_state::<mush_propagation::PropagationStore>()?
                        .envelope(*id)
                });
                patch
                    .observations
                    .push(json!({"slot":member.slot,"transport":envelope}));
                let Some(envelope) = envelope else {
                    // Retention or a held transport document is never evidence
                    // of delivery. Keep the last confirmed standing objective.
                    book.discard_pending(member.slot, pending.sequence, now)
                        .map_err(|e| e.to_string())?;
                    orders.outbound.remove(&member.slot);
                    continue;
                };
                if envelope.kind() != DispatchKind::Detachment
                    || envelope.issuer() != pending.issuer
                    || Some(envelope.recipient()) != pending.recipient.controller.character()
                    || envelope.dispatch_ref().is_none_or(|r| {
                        r.key != field_orders::message_key(old.id, pending.sequence, member.slot)
                    })
                {
                    return Err("Field transport differs from its source message.".into());
                }
                if matches!(
                    envelope.state(),
                    EnvelopeState::Expired | EnvelopeState::Superseded
                ) {
                    book.discard_pending(member.slot, pending.sequence, now)
                        .map_err(|e| e.to_string())?;
                    orders.outbound.remove(&member.slot);
                    continue;
                }
                if !matches!(
                    envelope.state(),
                    EnvelopeState::Delivered | EnvelopeState::Acknowledged
                ) {
                    continue;
                }
                if !roster.member_available(member.slot) || !allowed(pending.issuer, member.unit) {
                    book.discard_pending(member.slot, pending.sequence, now)
                        .map_err(|e| e.to_string())?;
                    orders.outbound.remove(&member.slot);
                    continue;
                }
                book.deliver(&roster, member.slot, pending.sequence, now, |m| {
                    allowed(pending.issuer, m.unit)
                })
                .map_err(|e| e.to_string())?;
                orders.received.insert(member.slot, envelope.id());
                orders.outbound.remove(&member.slot);
                orders.replies.remove(&member.slot);
            }
        }
        Act::Respond {
            slot,
            sequence,
            response,
        } => {
            book.respond(*slot, *sequence, actor, *response, now)
                .map_err(|e| e.to_string())?;
            let original = *orders
                .received
                .get(slot)
                .ok_or("Delivered transport unavailable.")?;
            let bytes = u32::try_from(
                serde_json::to_vec(&book.delivered(*slot).and_then(|d| d.acknowledgement))
                    .map_err(|e| e.to_string())?
                    .len(),
            )
            .map_err(|_| "Field reply too large.")?;
            orders.replies.remove(slot);
            patch.transport = Some(Box::new(Transport::Reply {
                original,
                reference: DispatchRef {
                    version: 1,
                    key: format!(
                        "{}:reply:{}",
                        field_orders::message_key(old.id, *sequence, *slot),
                        field_orders::response_word(*response)
                    ),
                    bytes,
                    origin: None,
                    fallback: None,
                },
            }));
        }
    }
    orders.book = book.checkpoint();
    let mut next = old.clone();
    next.command = Some(Box::new(roster.checkpoint()));
    next.orders = Some(Box::new(orders));
    patch.record = Some(Box::new(Write {
        before: Some(old),
        after: next,
    }));
    Ok(patch)
}

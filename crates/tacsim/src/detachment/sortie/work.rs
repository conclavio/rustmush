//! A bounded causal link from an actual surface action to a delivered NPC task.
//! This is neither permission nor a report endpoint for a player/controller.
use super::runtime::error as bad;
use super::{FlightKey, OperationId, SurfaceCrew};
use mush_tactics::detachment::Slot;
use serde::{Deserialize, Serialize};

pub(crate) fn sequence(
    parameters: &serde_json::Map<String, serde_json::Value>,
) -> Result<Option<u64>, mush_core::ActionError> {
    parameters
        .get("field_order")
        .map(|value| {
            let text = value
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()))
                .ok_or_else(|| bad("Invalid field-order sequence."))?;
            text.parse::<u64>()
                .ok()
                .filter(|s| *s > 0)
                .ok_or_else(|| bad("Invalid field-order sequence."))
        })
        .transpose()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "act",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FieldStep {
    Act(crate::surface::excursion::ActKind),
    Stop,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldWork {
    pub operation: OperationId,
    pub flight: FlightKey,
    pub slot: Slot,
    pub sequence: u64,
    pub ordinal: u32,
    pub step: FieldStep,
    /// Set only by an actual positive extraction receipt for this task.
    pub mined: bool,
}
impl FieldWork {
    pub(crate) fn check(&self, crew: &SurfaceCrew, ordinal: u32) -> Result<(), String> {
        if self.operation != crew.operation
            || self.flight != crew.flight
            || self.sequence == 0
            || self.ordinal == 0
            || self.ordinal > ordinal
        {
            return Err("Field work must identify its actual accepted flight and action.".into());
        }
        Ok(())
    }
}

pub(crate) struct WorkAdmission<'a> {
    pub actor: mush_core::ObjId,
    pub crew: &'a SurfaceCrew,
    pub sequence: u64,
    pub ordinal: u32,
    pub step: FieldStep,
    pub target: Option<[i64; 2]>,
    pub family: Option<crate::surface::geology::OreFamily>,
    pub previous: Option<&'a FieldWork>,
}

/// The eventual surface branch must succeed before this staged observation can
/// commit. Its action and the source order share the same Tacsim transaction.
pub(crate) fn admit(
    ctx: &mush_core::PrepareCtx<'_>,
    txn: &mut mush_core::ActionTxn<'_>,
    input: WorkAdmission<'_>,
) -> Result<FieldWork, mush_core::ActionError> {
    use super::FieldObjective;
    use crate::surface::excursion::ActKind;
    use mush_tactics::detachment::{
        Controller, DetachmentRoster, Directive, ExecutionObservation, OrderBook,
    };
    let world = ctx.world().map_err(bad)?;
    if input.actor != input.crew.operator
        || !super::facts::live(world, input.actor, mush_core::ObjKind::Npc)
    {
        return Err(bad("Only the actual NPC recipient can execute this task."));
    }
    let h = world
        .module_state::<super::SortieHandles>()
        .ok_or_else(|| bad("Field history unavailable."))?;
    let mut op = super::prepare::operation(&mut &mut *txn, h, input.crew.operation).map_err(bad)?;
    if op.ended.is_some() {
        return Err(bad("The field operation ended."));
    }
    let roster = DetachmentRoster::restore(
        op.command
            .as_deref()
            .ok_or_else(|| bad("Field command unavailable."))?,
    )
    .map_err(bad)?;
    let member = roster
        .members()
        .iter()
        .find(|m| {
            m.unit == input.crew.flight.unit
                && matches!(m.controller,Controller::Npc{character} if character==input.actor)
        })
        .ok_or_else(|| bad("Not this task's accepted NPC operator."))?;
    let orders = op
        .orders
        .as_deref()
        .ok_or_else(|| bad("No delivered field task."))?;
    let mut book = OrderBook::restore(&orders.book, &roster).map_err(bad)?;
    let delivered = book
        .delivered(member.slot)
        .filter(|d| d.order.sequence == input.sequence && d.completed_at_ms.is_none())
        .cloned()
        .ok_or_else(|| bad("The delivered field task changed."))?;
    let now = u64::try_from(world.world_ic_now().at)
        .ok()
        .and_then(|n| n.checked_mul(1000))
        .ok_or_else(|| bad("Field clock unavailable."))?;
    if now < delivered.delivered_at_ms
        || delivered.order.expires_at_ms.is_some_and(|end| now >= end)
    {
        return Err(bad("The field task expired."));
    }
    let context = orders
        .flights
        .get(&member.slot)
        .and_then(|f| f.standing.as_ref())
        .filter(|f| f.sequence == input.sequence && f.flight == input.crew.flight)
        .ok_or_else(|| bad("This message has no matching field flight."))?;
    let (_, flight) =
        super::prepare::current(&mut &mut *txn, h, input.crew.flight.unit).map_err(bad)?;
    if flight.key != context.flight
        || flight.site != context.site
        || flight.operation != input.crew.operation
        || flight.operator != input.actor
    {
        return Err(bad("The task's field flight changed."));
    }
    let allowed = match (&delivered.order.directive, input.step) {
        (Directive::Cancel, FieldStep::Stop) => true,
        (Directive::Objective(payload), step) => match (&payload.objective, step) {
            (FieldObjective::Hold, FieldStep::Stop) => true,
            (
                FieldObjective::Advance { .. }
                | FieldObjective::Withdraw
                | FieldObjective::Mine { .. },
                FieldStep::Stop,
            ) => true,
            (FieldObjective::Advance { to }, FieldStep::Act(ActKind::Drive)) => {
                input.target == Some([i64::from(to.0), i64::from(to.1)])
            }
            (FieldObjective::Withdraw, FieldStep::Act(ActKind::Recover | ActKind::Unload)) => true,
            (FieldObjective::Mine { family }, FieldStep::Act(ActKind::Mine)) => {
                family.is_none() || *family == input.family
            }
            (
                FieldObjective::Mine { .. },
                FieldStep::Act(
                    ActKind::Scan
                    | ActKind::Sound
                    | ActKind::Drive
                    | ActKind::Recover
                    | ActKind::Unload,
                ),
            ) => true,
            _ => false,
        },
        _ => false,
    };
    if !allowed {
        return Err(bad("This action does not attempt the delivered task."));
    }
    let same = input.previous.filter(|old| {
        old.operation == input.crew.operation
            && old.flight == input.crew.flight
            && old.slot == member.slot
            && old.sequence == input.sequence
    });
    let step = if input.step == FieldStep::Stop
        && same.is_some_and(|old| {
            old.step == FieldStep::Act(ActKind::Mine) && old.ordinal == input.ordinal
        }) {
        FieldStep::Act(ActKind::Mine)
    } else {
        input.step
    };
    let work = FieldWork {
        operation: input.crew.operation,
        flight: input.crew.flight,
        slot: member.slot,
        sequence: input.sequence,
        ordinal: input.ordinal,
        step,
        mined: same.is_some_and(|old| old.mined),
    };
    if matches!(delivered.order.directive, Directive::Objective(_)) {
        book.observe_execution(
            member.slot,
            input.sequence,
            ExecutionObservation::Attempted,
            now,
        )
        .map_err(bad)?;
        op.orders
            .as_mut()
            .ok_or_else(|| bad("Field orders unavailable."))?
            .book = book.checkpoint();
        op.validate().map_err(bad)?;
        txn.replace(&h.operations, &op.id, op.clone())
            .map_err(bad)?;
    }
    Ok(work)
}

/// Observe only the actual source-owned receipt. A stale receipt cannot finish
/// a replacement task, and an acknowledgement is never an execution receipt.
pub(crate) fn finish(
    ctx: &mush_core::PrepareCtx<'_>,
    txn: &mut mush_core::ActionTxn<'_>,
    record: &mut crate::surface::excursion::RigRecord,
) -> Result<(), mush_core::ActionError> {
    use super::FieldObjective;
    use crate::surface::excursion::{ActKind, ActResult};
    use mush_tactics::detachment::{DetachmentRoster, Directive, ExecutionObservation, OrderBook};
    let Some(work) = record.field_work.as_deref().cloned() else {
        return Ok(());
    };
    let Some(outcome) = record.last.as_ref().filter(|o| o.ordinal == work.ordinal) else {
        return Ok(());
    };
    let matches = match work.step {
        FieldStep::Act(kind) => kind == outcome.kind,
        FieldStep::Stop => {
            outcome.kind == ActKind::Mine && outcome.mined.as_ref().is_some_and(|m| m.end.is_some())
        }
    };
    if !matches || outcome.result != ActResult::Done {
        return Ok(());
    }
    let crew = record
        .field_crew
        .as_deref()
        .ok_or_else(|| bad("Field work lost its operator."))?;
    if outcome.operator != crew.operator {
        return Err(bad("Field receipt belongs to another operator."));
    }
    let world = ctx.world().map_err(bad)?;
    let h = world
        .module_state::<super::SortieHandles>()
        .ok_or_else(|| bad("Field history unavailable."))?;
    let mut op = super::prepare::operation(&mut &mut *txn, h, work.operation).map_err(bad)?;
    let roster = DetachmentRoster::restore(
        op.command
            .as_deref()
            .ok_or_else(|| bad("Field command unavailable."))?,
    )
    .map_err(bad)?;
    let orders = op
        .orders
        .as_deref()
        .ok_or_else(|| bad("Field orders unavailable."))?;
    let mut book = OrderBook::restore(&orders.book, &roster).map_err(bad)?;
    let Some(delivered) = book
        .delivered(work.slot)
        .filter(|d| d.order.sequence == work.sequence && d.completed_at_ms.is_none())
    else {
        return Ok(());
    };
    if delivered.order.recipient.unit != record.tv
        || delivered.order.recipient.controller.character() != Some(crew.operator)
        || orders
            .flights
            .get(&work.slot)
            .and_then(|f| f.standing.as_ref())
            .is_none_or(|f| f.sequence != work.sequence || f.flight != work.flight)
    {
        return Ok(());
    }
    let Directive::Objective(payload) = &delivered.order.directive else {
        return Ok(());
    };
    let mined = work.mined
        || (work.step == FieldStep::Act(ActKind::Mine)
            && outcome.mined.as_ref().is_some_and(|m| m.tonnes > 0));
    let completed = match &payload.objective {
        FieldObjective::Advance { to } => {
            outcome.kind == ActKind::Drive
                && outcome.arrived == Some([i64::from(to.0), i64::from(to.1)])
        }
        FieldObjective::Withdraw => outcome.kind == ActKind::Recover,
        FieldObjective::Mine { family } => {
            mined
                && outcome.kind == ActKind::Unload
                && outcome.stays.values().all(|n| *n == 0)
                && outcome.moved.iter().any(|(good, n)| {
                    *n > 0
                        && family
                            .is_none_or(|f| good.as_str() == crate::surface::opening::ore_good(f))
                })
        }
        _ => false,
    };
    if let Some(link) = record.field_work.as_mut() {
        link.mined = mined;
    }
    if completed {
        let now = u64::try_from(world.world_ic_now().at)
            .ok()
            .and_then(|n| n.checked_mul(1000))
            .ok_or_else(|| bad("Field clock unavailable."))?;
        book.observe_execution(
            work.slot,
            work.sequence,
            ExecutionObservation::Completed,
            now,
        )
        .map_err(bad)?;
        op.orders
            .as_mut()
            .ok_or_else(|| bad("Field orders unavailable."))?
            .book = book.checkpoint();
        op.validate().map_err(bad)?;
        txn.replace(&h.operations, &op.id, op.clone())
            .map_err(bad)?;
    }
    Ok(())
}

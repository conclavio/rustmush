use super::{
    model::*,
    runtime::{self, Assignment, Command},
};
use crate::detachment::planning::{DraftRef, PlanningHandles};
use mush_core::{CmdError, Ctx, ObjId};
use mush_org::unit_authority::{
    UnitCapability,
    reader::{UnitAuthorityRequest, UnitAuthorityResponse},
};
use mush_tactics::detachment::planning::{PlanId, PlannedController};

pub(crate) const SWITCHES: &[&str] = &[
    "seat",
    "launch",
    "recover",
    "cancel-launch",
    "operation",
    "history",
    "end",
    "refresh",
    "field",
    "orders",
    "command-grants",
];
pub(crate) fn handles(ctx: &Ctx<'_>) -> bool {
    SWITCHES.iter().any(|s| ctx.switches.has(s))
}
fn error() -> CmdError {
    CmdError::msg(
        "Use +sortie/launch <draft>:<activation revision>=all|<slot>; +sortie/operation shows saved progress.",
    )
}
fn number(raw: &str) -> Result<u64, CmdError> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(error());
    }
    raw.parse().ok().filter(|n| *n > 0).ok_or_else(error)
}
pub(crate) fn id(raw: &str, actor: ObjId) -> Result<OperationId, CmdError> {
    let fields = raw.trim().split(':').collect::<Vec<_>>();
    let (organizer, draft, activation) = match fields.as_slice() {
        [draft, revision] => (actor, *draft, *revision),
        [organizer, draft, revision] => (
            ObjId(u32::try_from(number(organizer.trim_start_matches('#'))?).map_err(|_| error())?),
            *draft,
            *revision,
        ),
        _ => return Err(error()),
    };
    let result = OperationId {
        organizer,
        draft: PlanId(number(draft)?),
        activation: number(activation)?,
    };
    result.key().map_err(CmdError::msg)?;
    Ok(result)
}
fn slot(raw: &str) -> Result<u16, CmdError> {
    u16::try_from(number(raw.trim())?)
        .ok()
        .filter(|n| (1..=6).contains(n))
        .ok_or_else(error)
}
fn line(ctx: &mut Ctx<'_>, message: impl Into<String>) {
    ctx.out
        .sys_to_actor(ctx.actor, "tacsim.detachment", message.into())
}
fn readable(ctx: &Ctx<'_>, op: &OperationRecord) -> bool {
    ctx.actor == op.id.organizer
        || op.roster.plan.slots.iter().any(
            |s| matches!(s.controller,PlannedController::Player{character} if character==ctx.actor),
        )
}
pub(super) fn read(ctx: &Ctx<'_>, id: OperationId) -> Result<OperationRecord, CmdError> {
    let h = ctx
        .w
        .module_state::<runtime::SortieHandles>()
        .ok_or_else(error)?;
    let op = ctx
        .action_state(&h.operations, &id)
        .map_err(|_| error())?
        .cloned()
        .filter(|o| o.id == id && readable(ctx, o))
        .ok_or_else(error)?;
    op.validate().map_err(CmdError::msg)?;
    Ok(op)
}
pub(crate) fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.iter().count() != 1 || ctx.args.raw().len() > 256 {
        return Err(error());
    }
    let raw = ctx.args.raw().trim().to_owned();
    if ctx.switches.has("history") {
        return history(ctx, &raw);
    }
    let (selector, selection) = raw
        .split_once('=')
        .map_or((raw.as_str(), None), |(left, right)| (left, Some(right)));
    let id = id(selector, ctx.actor)?;
    if ctx.switches.has("command-grants") {
        if selection.is_some() {
            return Err(error());
        }
        return command_grants(ctx, id);
    }
    if ctx.switches.has("orders") {
        if selection.is_some() {
            return Err(error());
        }
        let op = read(ctx, id)?;
        return super::field_commands::report(ctx, &op).map_err(CmdError::msg);
    }
    if ctx.switches.has("refresh") || ctx.switches.has("field") {
        if selection.is_some() {
            return Err(error());
        }
        let op = read(ctx, id)?;
        if ctx.switches.has("refresh") {
            return super::field::refresh(ctx, id).map_err(CmdError::msg);
        }
        if let Some(saved) = &op.command {
            let roster = mush_tactics::detachment::roster::DetachmentRoster::restore(saved)
                .map_err(|e| CmdError::msg(e.to_string()))?;
            line(
                ctx,
                format!(
                    "Last confirmed field command: detachment {}, revision {}, term {}; {}.",
                    roster.id().0,
                    roster.revision(),
                    roster.term(),
                    roster.leader().map_or_else(
                        || "no leader".into(),
                        |l| format!("slot {}, #{} leads", l.slot.get(), l.character.0)
                    )
                ),
            );
        } else {
            line(ctx, "Field command has not yet been refreshed.");
        }
        line(
            ctx,
            format!(
                "Refresh current field eligibility: +sortie/refresh {}:{}:{}",
                id.organizer.0, id.draft.0, id.activation
            ),
        );
        return Ok(());
    }
    if ctx.switches.has("operation") {
        if selection.is_some() {
            return Err(error());
        }
        return report(ctx, id);
    }
    if id.organizer != ctx.actor && !ctx.switches.has("recover") {
        return Err(error());
    }
    if ctx.switches.has("launch") {
        return launch(ctx, id, selection.unwrap_or("all"));
    }
    if ctx.switches.has("seat") {
        return super::seating::seat(ctx, id, slot(selection.ok_or_else(error)?)?);
    }
    let command = if ctx.switches.has("end") {
        if selection.is_some() {
            return Err(error());
        }
        Command::End { operation: id }
    } else {
        let slot = slot(selection.ok_or_else(error)?)?;
        let op = read(ctx, id)?;
        let unit = op
            .roster
            .plan
            .slots
            .iter()
            .find(|s| s.slot.get() == slot)
            .and_then(|s| s.unit)
            .ok_or_else(error)?;
        if ctx.switches.has("recover") {
            Command::Recover {
                operation: id,
                slot,
                unit,
            }
        } else {
            Command::Cancel {
                operation: id,
                slot,
            }
        }
    };
    runtime::submit(ctx, command).map_err(CmdError::msg)
}
fn command_grants(ctx: &mut Ctx<'_>, id: OperationId) -> Result<(), CmdError> {
    let h = ctx
        .w
        .module_state::<runtime::SortieHandles>()
        .ok_or_else(error)?;
    let roster=ctx.action_state(&h.active,&id.reference()).map_err(|_|error())?.cloned()
        .filter(|r|r.revision==id.activation&&r.active()
            &&(ctx.actor==r.organizer||r.plan.slots.iter().any(|s|matches!(s.controller,PlannedController::Player{character} if character==ctx.actor))))
        .ok_or_else(error)?;
    roster.validate().map_err(CmdError::msg)?;
    let binding = super::field::mission_binding(&roster).map_err(CmdError::msg)?;
    line(
        ctx,
        "Each unit's owner or authorized lender can issue its printed mission-role grant. The role follows the eligible current leader; it grants no crew, deployment or onward-delegation permission.",
    );
    for member in &roster.plan.slots {
        let draft = mush_org::unit_authority::reader::UnitGrantDraft {
            grantee: mush_org::unit_authority::UnitGrantee::CurrentLeader(binding.clone()),
            capability: UnitCapability::CommandAssignedAi,
            scope: mush_org::unit_authority::UnitScope::Units {
                units: std::collections::BTreeSet::from([member.unit.ok_or_else(error)?]),
            },
            valid_until: None,
            delegable: false,
            recovery: mush_org::unit_authority::RecoveryPolicy::None,
            parent: None,
        };
        line(
            ctx,
            format!(
                "Slot {}: @unitgrant/json {}",
                member.slot.get(),
                serde_json::to_string(&draft).map_err(|_| error())?
            ),
        );
    }
    Ok(())
}
fn launch(ctx: &mut Ctx<'_>, id: OperationId, selection: &str) -> Result<(), CmdError> {
    let h = ctx
        .w
        .module_state::<runtime::SortieHandles>()
        .cloned()
        .ok_or_else(error)?;
    let roster = ctx
        .action_state(
            &h.active,
            &DraftRef {
                organizer: ctx.actor,
                id: id.draft,
            },
        )
        .map_err(|_| error())?
        .cloned()
        .filter(|r| r.active() && OperationId::of(r) == id)
        .ok_or_else(error)?;
    roster.validate().map_err(CmdError::msg)?;
    let selected = if selection.trim() == "all" {
        None
    } else {
        Some(slot(selection)?)
    };
    let mut slots = Vec::new();
    for row in &roster.plan.slots {
        if selected.is_some_and(|s| s != row.slot.get()) {
            continue;
        }
        let unit = row.unit.ok_or_else(error)?;
        if ctx
            .action_state(&h.heads, &unit)
            .map_err(|_| error())?
            .is_some()
        {
            let (_, flight) =
                super::prepare::current(&mut &*ctx, &h, unit).map_err(CmdError::msg)?;
            if flight.operation == id && matches!(flight.phase, Phase::Field | Phase::Returning) {
                line(
                    ctx,
                    format!(
                        "Slot {} already departed; it will not be launched again.",
                        row.slot.get()
                    ),
                );
                continue;
            }
        }
        slots.push(Assignment {
            slot: row.slot.get(),
            unit,
            operator: row.controller.character().ok_or_else(error)?,
        });
    }
    if slots.is_empty() {
        line(ctx, "No unlaunched members in that selection.");
        return Ok(());
    }
    runtime::submit(
        ctx,
        Command::Launch {
            operation: id,
            slots,
        },
    )
    .map_err(CmdError::msg)
}
pub(super) fn check_caller_rights(ctx: &Ctx<'_>, command: &Command) -> Result<(), String> {
    let checks: Vec<_> = match command {
        Command::Launch { slots, .. } => slots
            .iter()
            .map(|s| (s.unit, UnitCapability::Deploy))
            .collect(),
        Command::Recover { unit, .. } => vec![(*unit, UnitCapability::RecoverLoad)],
        _ => return Ok(()),
    };
    let reader = ctx
        .w
        .module_state::<PlanningHandles>()
        .and_then(|h| h.unit_authority_reader.as_ref())
        .ok_or("Unit authority unavailable.")?;
    for (unit, capability) in checks {
        if !matches!(
            ctx.read_owner(
                reader,
                UnitAuthorityRequest::Authorize {
                    unit,
                    capability,
                    delegate: false,
                    mission: None
                }
            ),
            Ok(UnitAuthorityResponse::Authority { allowed: true })
        ) {
            return Err("Current unit-use authority unavailable or denied.".into());
        }
    }
    Ok(())
}
fn report(ctx: &mut Ctx<'_>, id: OperationId) -> Result<(), CmdError> {
    let op = read(ctx, id)?;
    let h = ctx
        .w
        .module_state::<runtime::SortieHandles>()
        .cloned()
        .ok_or_else(error)?;
    line(
        ctx,
        format!(
            "{} operation {}:{}:{}; accepted draft revision {}.",
            if op.ended.is_some() { "Ended" } else { "Open" },
            id.organizer.0,
            id.draft.0,
            id.activation,
            op.roster.source_revision
        ),
    );
    for slot in &op.roster.plan.slots {
        let unit = slot.unit.ok_or_else(error)?;
        let flight = op
            .flights
            .iter()
            .rev()
            .find(|key| key.unit == unit)
            .map(|key| {
                ctx.action_state(&h.flights, key)
                    .map(|f| f.cloned())
                    .map_err(|_| error())
            })
            .transpose()?
            .flatten();
        if let Some(flight) = &flight {
            super::prepare::membership(&op, flight).map_err(CmdError::msg)?;
        }
        line(
            ctx,
            match flight {
                Some(flight) => format!(
                    "Slot {}: unit #{} — last confirmed {}; history: +sortie/history #{}:{}{}.",
                    slot.slot.get(),
                    unit.0,
                    flight.phase.label(),
                    unit.0,
                    flight.key.ordinal,
                    if op.ended.is_some() && flight.phase.open() {
                        "; unresolved, physical epoch remains open"
                    } else {
                        ""
                    }
                ),
                None => format!(
                    "Slot {}: unit #{} — no departure recorded for this operation.",
                    slot.slot.get(),
                    unit.0
                ),
            },
        );
    }
    line(
        ctx,
        "Pending departures retry when their current physical prerequisites are restored. Cancel only unlaunched members; ending an operation never recalls field units.",
    );
    Ok(())
}
fn history(ctx: &mut Ctx<'_>, raw: &str) -> Result<(), CmdError> {
    let (unit, ordinal) = raw.split_once(':').ok_or_else(error)?;
    let key = FlightKey {
        unit: ObjId(u32::try_from(number(unit.trim_start_matches('#'))?).map_err(|_| error())?),
        ordinal: number(ordinal)?,
    };
    let h = ctx
        .w
        .module_state::<runtime::SortieHandles>()
        .ok_or_else(error)?;
    let flight = ctx
        .action_state(&h.flights, &key)
        .map_err(|_| error())?
        .cloned()
        .filter(|f| f.key == key)
        .ok_or_else(error)?;
    let op = read(ctx, flight.operation)?;
    if !readable(ctx, &op) {
        return Err(error());
    }
    flight.validate().map_err(CmdError::msg)?;
    super::prepare::membership(&op, &flight).map_err(CmdError::msg)?;
    line(
        ctx,
        format!(
            "Unit #{} flight {}: {}; operation {}:{}:{}, slot {}, launcher #{}, operator #{}.",
            key.unit.0,
            key.ordinal,
            flight.phase.label(),
            flight.operation.organizer.0,
            flight.operation.draft.0,
            flight.operation.activation,
            flight.slot,
            flight.launcher.0,
            flight.operator.0
        ),
    );
    if let Some(epoch) = flight.epoch {
        line(
            ctx,
            format!(
                "Hull epoch {}; carrier #{}; opened at IC {}{}.",
                epoch.id,
                epoch.hull.0,
                epoch.opened_ic,
                if epoch.unrecorded {
                    "; black-box opening unrecorded"
                } else {
                    ""
                }
            ),
        );
    }
    if let Some(close) = flight.closed {
        line(
            ctx,
            format!(
                "Returned at IC {}; operator currently aboard at that confirmation: {}.",
                close.at.at,
                if close.operator_aboard {
                    "yes"
                } else {
                    "no; crew outcome unresolved"
                }
            ),
        );
    }
    Ok(())
}

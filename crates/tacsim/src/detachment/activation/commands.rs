//! Activation adapters never move a unit or manufacture an authority decision.
use super::runtime::{self, Operation};
use crate::detachment::crew::CrewDuty;
use crate::detachment::planning::DraftRef;
use mush_core::{CmdError, Ctx, ObjId};
use mush_tactics::detachment::planning::{PlanId, PlannedController};

pub(crate) const SWITCHES: &[&str] = &["chat", "activate", "deactivate", "active", "preflight"];
pub(crate) fn handles(ctx: &Ctx<'_>) -> bool {
    SWITCHES.iter().any(|s| ctx.switches.has(s))
}
fn fail() -> CmdError {
    CmdError::msg(
        "Detachment selection unavailable. Use +sortie/plans and +sortie/active <draft> for current references.",
    )
}
fn number(raw: &str) -> Result<u64, CmdError> {
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err(fail());
    }
    raw.parse().ok().filter(|value| *value > 0).ok_or_else(fail)
}
fn reference(raw: &str, actor: ObjId) -> Result<DraftRef, CmdError> {
    if let Some((owner, id)) = raw.split_once(':') {
        Ok(DraftRef {
            organizer: ObjId(
                u32::try_from(number(owner.trim_start_matches('#'))?).map_err(|_| fail())?,
            ),
            id: PlanId(number(id)?),
        })
    } else {
        Ok(DraftRef {
            organizer: actor,
            id: PlanId(number(raw)?),
        })
    }
}
fn fence(raw: &str) -> Result<(PlanId, u64), CmdError> {
    let (id, revision) = raw.split_once(':').ok_or_else(fail)?;
    Ok((PlanId(number(id)?), number(revision)?))
}
fn line(ctx: &mut Ctx<'_>, text: impl Into<String>) {
    ctx.out
        .sys_to_actor(ctx.actor, "tacsim.detachment", text.into());
}
pub(crate) fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("chat") {
        return crate::detachment::chat::command(ctx);
    }
    if ctx.switches.iter().count() != 1 || ctx.args.raw().len() > 256 {
        return Err(fail());
    }
    let raw = ctx.args.raw().trim().to_owned();
    if ctx.switches.has("preflight") {
        let (id, revision) = fence(&raw)?;
        return super::preflight::report(ctx, id, revision).map_err(CmdError::msg);
    }
    if ctx.switches.has("active") {
        let reference = reference(&raw, ctx.actor)?;
        let Some(record) = runtime::read(ctx, reference).map_err(|_| fail())? else {
            line(ctx, "No accepted roster for that draft.");
            return Ok(());
        };
        line(
            ctx,
            format!(
                "{} detachment for draft {}, activation revision {} (source draft revision {}, duty {:?}).",
                if record.active() {
                    "Prepared"
                } else {
                    "Closed"
                },
                record.draft.0,
                record.revision,
                record.source_revision,
                record.duty
            ),
        );
        for slot in &record.plan.slots {
            let controller = match slot.controller {
                PlannedController::Player { character } => format!("player #{}", character.0),
                PlannedController::Npc { character } => format!("NPC #{}", character.0),
                _ => return Err(fail()),
            };
            line(
                ctx,
                format!(
                    "Slot {}: unit #{} — {controller}.",
                    slot.slot.get(),
                    slot.unit.ok_or_else(fail)?.0
                ),
            );
            if record.active()
                && record.organizer == ctx.actor
                && matches!(slot.controller, PlannedController::Npc { .. })
            {
                line(
                    ctx,
                    format!(
                        "Arrange NPC station: +sortie/seat {}:{}={}",
                        record.draft.0,
                        record.revision,
                        slot.slot.get()
                    ),
                );
            }
        }
        if record.active() {
            line(
                ctx,
                format!(
                    "Detachment IC chat: +sortie/chat {}:{}:{}",
                    record.organizer.0, record.draft.0, record.revision
                ),
            );
        }
        if record.active() && record.organizer == ctx.actor {
            line(
                ctx,
                format!(
                    "Inspect launch: +sortie/preflight {}:{}",
                    record.draft.0, record.revision
                ),
            );
            line(
                ctx,
                format!(
                    "Release: +sortie/deactivate {}:{}",
                    record.draft.0, record.revision
                ),
            );
        }
        line(
            ctx,
            "These are accepted assignments. Use +sortie/plan for current presence; launch preflight rechecks permissions and control.",
        );
        return Ok(());
    }
    let operation = if ctx.switches.has("activate") {
        let (selector, purpose) = raw.split_once('=').ok_or_else(fail)?;
        let (id, expected_revision) = fence(selector.trim())?;
        let duty = match purpose.trim() {
            "mining" => CrewDuty::Mining,
            "combat" => CrewDuty::Combat,
            "recovery" => CrewDuty::Recovery,
            _ => return Err(fail()),
        };
        let before = runtime::read(
            ctx,
            DraftRef {
                organizer: ctx.actor,
                id,
            },
        )
        .map_err(|_| fail())?;
        Operation::Activate {
            id,
            expected_revision,
            before_activation: before.as_ref().map_or(0, |r| r.revision),
            duty,
        }
    } else {
        let (id, expected_activation) = fence(&raw)?;
        Operation::Deactivate {
            id,
            expected_activation,
        }
    };
    runtime::submit(ctx, operation).map_err(|_| fail())
}

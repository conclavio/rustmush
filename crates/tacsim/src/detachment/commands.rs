//! Live OOC detachment commands share the cockpit's ordinary line transport.
//! Every client prefill carries the roster and leadership fences it displayed.

use std::sync::Arc;

use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ModError, ObjId, Registrar, World};
use mush_tactics::detachment::{Directive, OrderTarget, Response, Slot};
use serde_json::Value;

use super::{PracticeObjective, live};
use crate::deliberation::{Constraints, Objective};
use crate::match_::cockpit::{self, ObjectiveWords, Seat, SquadWord};

const USAGE: &str =
    "Use the current roster's ORDER DT:<id>:<revision>:<term> <slot|ALL> <objective> prefill.";

pub(super) fn numbers<const N: usize>(word: &str) -> Result<[u64; N], String> {
    let mut fields = word.strip_prefix("DT:").ok_or(USAGE)?.split(':');
    let mut result = [0; N];
    for value in &mut result {
        let field = fields.next().ok_or(USAGE)?;
        if field.is_empty() || !field.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(USAGE.to_owned());
        }
        *value = field.parse().map_err(|_| USAGE.to_owned())?;
    }
    if fields.next().is_some() {
        return Err(USAGE.to_owned());
    }
    Ok(result)
}

/// Called only from the same cockpit handler that resolves the actor's seat.
pub fn order(ctx: &mut Ctx<'_>, seat: &Seat, rest: &str) -> Result<(), String> {
    let (fence, text) = rest.split_once(char::is_whitespace).ok_or(USAGE)?;
    let [detachment, revision, term] = numbers(fence)?;
    live::sync(ctx);
    crate::arena_bridge::flush_detachment_dispatches(ctx);
    let context =
        live::command_context(ctx.w, seat.driver).ok_or("No active practice detachment.")?;
    if context.roster.id().0 != detachment
        || context.roster.revision() != revision
        || context.roster.term() != term
    {
        return Err("Detachment changed; refresh the roster before ordering.".to_owned());
    }
    // Check the principal before resolving contacts, spots or target slots.
    if context
        .roster
        .leader()
        .is_none_or(|leader| leader.character != seat.driver)
    {
        return Err("Only the current detachment leader may issue orders.".to_owned());
    }
    let words = cockpit::parse_order(text).map_err(|error| error.to_string())?;
    let target = match words.squad {
        SquadWord::All => OrderTarget::All,
        SquadWord::Ordinal(slot) => {
            OrderTarget::Slot(Slot::new(slot).map_err(|error| error.to_string())?)
        }
        SquadWord::Id(_) => return Err(USAGE.to_owned()),
    };
    if let OrderTarget::Slot(slot) = target
        && context.roster.member(slot).is_none()
    {
        return Err("No such detachment slot.".to_owned());
    }
    let directive = match words.objective {
        ObjectiveWords::Release => Directive::Cancel,
        objective_words => {
            let objective = match objective_words {
                ObjectiveWords::Hold => Objective::Hold,
                ObjectiveWords::Withdraw => Objective::Withdraw,
                ObjectiveWords::Screen { bearing } => Objective::Screen {
                    facing_mdeg: bearing.saturating_mul(1_000),
                },
                ObjectiveWords::Advance { spot } => {
                    let (spots, _) = cockpit::spots_of(ctx.w, Some(context.match_id));
                    let (_, found) =
                        cockpit::resolve_spot(&spots, &spot).ok_or(cockpit::NO_SUCH_CONTACT)?;
                    Objective::Advance {
                        to: (found.x, found.y),
                    }
                }
                ObjectiveWords::Engage { contact } => {
                    let target_hint = if let Some(word) = contact {
                        let held = cockpit::last_frame(ctx.w, seat.driver)
                            .ok_or(cockpit::NO_SUCH_CONTACT)?;
                        let rows = cockpit::frame_contacts_held(&held.frame, &held.ranges);
                        let id = cockpit::resolve_designator(&rows, &word)
                            .ok_or(cockpit::NO_SUCH_CONTACT)?;
                        if rows.iter().any(|row| row.id == id && row.friendly) {
                            None
                        } else {
                            Some(id)
                        }
                    } else {
                        None
                    };
                    Objective::Engage { target_hint }
                }
                ObjectiveWords::Support { callsign } => {
                    let record = ctx
                        .w
                        .module_state::<crate::match_::MatchBook>()
                        .and_then(|book| book.get(context.match_id))
                        .ok_or("No active practice match.")?;
                    let of = record
                        .callsigns
                        .iter()
                        .find(|(unit, designator)| {
                            context
                                .roster
                                .members()
                                .iter()
                                .any(|member| member.unit == **unit)
                                && designator.to_string().eq_ignore_ascii_case(&callsign)
                        })
                        .map(|(unit, _)| *unit)
                        .ok_or(cockpit::NO_SUCH_FRIENDLY)?;
                    Objective::Support { of }
                }
                ObjectiveWords::Release => return Err(USAGE.to_owned()),
            };
            Directive::Objective(
                PracticeObjective::new(
                    &context.roster,
                    objective,
                    Constraints {
                        hold_fire: words.hold_fire,
                        max_range_band: None,
                    },
                    context.window,
                )
                .map_err(|error| error.to_string())?,
            )
        }
    };
    live::issue(
        ctx,
        live::LiveOrderRequest {
            detachment,
            revision,
            term,
            target,
            directive,
            expires_after_ms: None,
        },
    )
    .map_err(|error| error.to_string())?;
    crate::arena_bridge::flush_detachment_dispatches(ctx);
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.detachment",
        "Order sent; delivery and acknowledgement are reported separately.",
    );
    refresh(ctx);
    Ok(())
}

fn respond(ctx: &mut Ctx<'_>, response: Response) -> Result<(), CmdError> {
    let raw = ctx.args.raw().trim().to_owned();
    if let Some(result) = super::sortie::field_response(ctx, &raw, response) {
        return result.map_err(CmdError::msg);
    }
    if cockpit::flying(ctx.w, ctx.actor).is_none() {
        return Err(CmdError::msg(
            "You must be at your assigned cockpit to respond.",
        ));
    }
    let text = ctx.args.raw().trim().to_owned();
    let (fence, sequence) = text.split_once(char::is_whitespace).ok_or_else(|| {
        CmdError::msg("Use WILCO or NEGATIVE with the delivered order's current prefill.")
    })?;
    let [detachment, revision] = numbers(fence).map_err(CmdError::msg)?;
    let sequence: u64 = sequence
        .trim()
        .parse()
        .map_err(|_| CmdError::msg("Invalid order sequence."))?;
    live::sync(ctx);
    live::respond(ctx, detachment, revision, sequence, response)
        .map_err(|error| CmdError::msg(error.to_string()))?;
    crate::arena_bridge::flush_detachment_dispatches(ctx);
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.detachment",
        match response {
            Response::Wilco => "WILCO recorded. Your controls remain yours.",
            Response::Negative => "NEGATIVE recorded. Your controls remain yours.",
        },
    );
    refresh(ctx);
    Ok(())
}

/// Add world-owned authority to the same frame used by query, push and commands.
pub fn enrich(w: &World, actor: ObjId, mut payload: Value) -> Value {
    if let Some(object) = payload.as_object_mut() {
        if let Some(detachment) = live::projection(w, actor) {
            object.insert("detachment".to_owned(), detachment);
        } else {
            object.remove("detachment");
        }
    }
    payload
}

fn refresh(ctx: &mut Ctx<'_>) {
    let actor = ctx.actor;
    let Some(held) = cockpit::last_frame(ctx.w, actor) else {
        return;
    };
    let data = enrich(
        ctx.w,
        actor,
        cockpit::status_frame_held(held.match_id, &held.frame, &held.ranges),
    );
    ctx.out.push(
        mush_core::Audience::Actor(actor),
        mush_proto::OutputEvent::Panel {
            feed: crate::arena_bridge::FRAME_FEED.into(),
            kind: mush_proto::PanelFrameKind::Keyframe,
            data,
        },
    );
}

fn roster(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.iter().count() > 1 {
        return Err(CmdError::msg("Choose one +sortie operation."));
    }
    if super::sortie::handles(ctx) {
        return super::sortie::command(ctx);
    }
    if super::activation::handles(ctx) {
        return super::activation::command(ctx);
    }
    if super::crew::handles(ctx) {
        return super::crew::command(ctx);
    }
    if super::planning::commands::handles(ctx) {
        return super::planning::commands::command(ctx);
    }
    if ctx.switches.has("practice") {
        live::enable(ctx).map_err(|error| CmdError::msg(error.to_string()))?;
    }
    live::sync(ctx);
    crate::arena_bridge::flush_detachment_dispatches(ctx);
    let projection = live::projection(ctx.w, ctx.actor)
        .ok_or_else(|| CmdError::msg("Use +sortie/practice while seated in a running OOC match to form a practice detachment."))?;
    let leader = projection
        .get("leader_slot")
        .and_then(Value::as_u64)
        .map_or_else(|| "none".to_owned(), |slot| slot.to_string());
    ctx.out.sys_to_actor(
        ctx.actor,
        "tacsim.detachment",
        format!("Practice detachment — leader slot {leader}."),
    );
    if let Some(slots) = projection.get("slots").and_then(Value::as_array) {
        for slot in slots {
            let ordinal = slot.get("slot").and_then(Value::as_u64).unwrap_or(0);
            let callsign = slot.get("callsign").and_then(Value::as_str).unwrap_or("?");
            let prefill = slot
                .get("order_prefill")
                .and_then(Value::as_str)
                .unwrap_or("");
            ctx.out.sys_to_actor(
                ctx.actor,
                "tacsim.detachment",
                format!("{ordinal}: {callsign} — {prefill}<objective>"),
            );
        }
    }
    if let Some(prefill) = projection.get("all_order_prefill").and_then(Value::as_str) {
        ctx.out.sys_to_actor(
            ctx.actor,
            "tacsim.detachment",
            format!("All slots: {prefill}<objective>"),
        );
    }
    for key in ["wilco_command", "negative_command"] {
        if let Some(command) = projection.get(key).and_then(Value::as_str) {
            ctx.out
                .sys_to_actor(ctx.actor, "tacsim.detachment", command);
        }
    }
    refresh(ctx);
    Ok(())
}

pub fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    let switches = ["practice", "roster"]
        .into_iter()
        .chain(super::planning::commands::SWITCHES.iter().copied())
        .chain(super::crew::SWITCHES.iter().copied())
        .chain(super::activation::SWITCHES.iter().copied())
        .chain(super::sortie::SWITCHES.iter().copied())
        .map(Into::into)
        .collect();
    reg.command(CommandSpec {
        name: "+sortie".into(), switches, shape: ArgShape::Raw,
        help: "+sortie/crew lists NPC hiring terms in your port room; +sortie/hire accepts them and +sortie/contract shows your agreement. +sortie/board <npc> arranges boarding for your hired crew at your ship's departure port. +sortie/plans manages private presets and mission drafts; +sortie/invitations lists assigned plans. Use +sortie/plan for accept/ready commands. +sortie/activate <draft>:<revision>=mining|combat|recovery reserves complete assignments; +sortie/active shows them and +sortie/deactivate releases them. +sortie/preflight <draft>:<activation revision> inspects current launch facts. +sortie/seat <draft>:<activation>=slot arranges an accepted NPC unit station; +sortie/launch <draft>:<activation>=all|slot starts physical deployment; +sortie/recover <draft>:<activation>=slot returns a mobile unit at the ramp. +sortie/cancel-launch cancels a pending departure; +sortie/end ends the operation without recalling field units. +sortie/operation shows confirmed deployment history and +sortie/history #unit:flight shows a departure. +sortie/practice and +sortie/roster manage the OOC group.".into(),
        ..CommandSpec::default()
    }, Arc::new(roster))?;
    for (name, response) in [("wilco", Response::Wilco), ("negative", Response::Negative)] {
        reg.command(CommandSpec { name: name.into(), shape: ArgShape::Raw,
            help: "Respond to a delivered detachment order using its current roster prefill. A response never moves, aims or fires your unit.".into(),
            ..CommandSpec::default()
        }, Arc::new(move |ctx: &mut Ctx<'_>| respond(ctx, response)))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fences_are_complete_unsigned_and_bounded() {
        assert_eq!(numbers::<3>("DT:7:8:9"), Ok([7, 8, 9]));
        for invalid in [
            "DT:7:8",
            "DT:7:8:9:10",
            "DT:7:-8:9",
            "DT:7::9",
            "DT:7:18446744073709551616:9",
            "7:8:9",
        ] {
            assert!(numbers::<3>(invalid).is_err());
        }
    }
}

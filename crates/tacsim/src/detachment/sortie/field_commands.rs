//! Field clients use the same fenced ORDER/WILCO/NEGATIVE line vocabulary.
use super::{
    field_orders::{self, Act, FieldObjective, FieldPayload},
    model::*,
    prepare,
    runtime::{self, Command, SortieHandles},
};
use crate::match_::cockpit::{ObjectiveWords, SquadWord, parse_order};
use mush_core::{Ctx, PrincipalId};
use mush_tactics::detachment::{Directive, OrderBook, OrderTarget, Response, Slot};

fn own(ctx: &Ctx<'_>, detachment: u64, revision: u64) -> Option<(SortieHandles, OperationRecord)> {
    let h = ctx.w.module_state::<SortieHandles>()?.clone();
    let reservation = ctx.action_state(&h.reservations, &ctx.actor).ok()??;
    if reservation.validate().is_err()
        || reservation.detachment != detachment
        || reservation.revision != revision
    {
        return None;
    }
    let id = OperationId {
        organizer: reservation.organizer,
        draft: reservation.draft,
        activation: reservation.revision,
    };
    let op = prepare::operation(&mut &*ctx, &h, id).ok()?;
    Some((h, op))
}
fn submit(
    ctx: &mut Ctx<'_>,
    h: &SortieHandles,
    op: &OperationRecord,
    act: Act,
) -> Result<(), String> {
    let request = field_orders::request(ctx, h, op, ctx.actor, act)?;
    runtime::submit(ctx, Command::Message(Box::new(request)))
}
pub(crate) fn field_order(ctx: &mut Ctx<'_>, raw: &str) -> Option<Result<(), String>> {
    let (fence, text) = raw.split_once(char::is_whitespace)?;
    let [detachment, revision, term] = crate::detachment::commands::numbers(fence).ok()?;
    let (h, op) = own(ctx, detachment, revision)?;
    Some(issue(ctx, &h, &op, revision, term, text))
}
fn issue(
    ctx: &mut Ctx<'_>,
    h: &SortieHandles,
    op: &OperationRecord,
    revision: u64,
    term: u64,
    text: &str,
) -> Result<(), String> {
    if text.len() > 256 {
        return Err("Field order is too long.".into());
    }
    let (text, expires_at_ms) = if let Some((text, seconds)) = text.rsplit_once(" EXPIRES ") {
        let seconds: u64 = seconds
            .parse()
            .map_err(|_| "EXPIRES needs a positive number of IC seconds.")?;
        if seconds == 0 {
            return Err("EXPIRES needs a positive number of IC seconds.".into());
        }
        let now = u64::try_from(ctx.w.world_ic_now().at).map_err(|_| "Field clock unavailable.")?;
        (
            text,
            Some(
                now.checked_add(seconds)
                    .and_then(|t| t.checked_mul(1000))
                    .ok_or("Field expiry exhausted.")?,
            ),
        )
    } else {
        (text, None)
    };
    let mut mining = text.split_whitespace();
    if let (Some(target), Some(verb)) = (mining.next(), mining.next())
        && verb.eq_ignore_ascii_case("MINE")
    {
        let target = if target.eq_ignore_ascii_case("ALL") {
            OrderTarget::All
        } else {
            OrderTarget::Slot(
                Slot::new(
                    target
                        .parse()
                        .map_err(|_| "Use a numbered field slot or ALL.")?,
                )
                .map_err(|e| e.to_string())?,
            )
        };
        let family = mining
            .next()
            .map(|word| {
                crate::surface::geology::family_of_word(word)
                    .ok_or("MINE needs a known ore family.")
            })
            .transpose()?;
        if mining.next().is_some() {
            return Err("Use MINE [family].".into());
        }
        return submit(
            ctx,
            h,
            op,
            Act::Issue {
                revision,
                term,
                target,
                directive: Directive::Objective(FieldPayload {
                    objective: FieldObjective::Mine { family },
                    hold_fire: false,
                }),
                expires_at_ms,
            },
        );
    }
    let words = parse_order(text).map_err(|e| e.to_string())?;
    let target = match words.squad {
        SquadWord::All => OrderTarget::All,
        SquadWord::Ordinal(n) => OrderTarget::Slot(Slot::new(n).map_err(|e| e.to_string())?),
        SquadWord::Id(_) => return Err("Use a numbered field slot or ALL.".into()),
    };
    let objective = match words.objective {
        ObjectiveWords::Release => None,
        ObjectiveWords::Hold => Some(FieldObjective::Hold),
        ObjectiveWords::Withdraw => Some(FieldObjective::Withdraw),
        ObjectiveWords::Screen { bearing } => Some(FieldObjective::Screen {
            bearing_mdeg: bearing * 1000,
        }),
        ObjectiveWords::Advance { spot } => {
            let (x, y) = spot
                .split_once(',')
                .ok_or("ADVANCE needs a field cell x,y.")?;
            Some(FieldObjective::Advance {
                to: (
                    x.trim().parse().map_err(|_| "Invalid field cell.")?,
                    y.trim().parse().map_err(|_| "Invalid field cell.")?,
                ),
            })
        }
        ObjectiveWords::Support { callsign } => Some(FieldObjective::Support {
            slot: Slot::new(
                callsign
                    .parse()
                    .map_err(|_| "SUPPORT needs an accepted slot number.")?,
            )
            .map_err(|e| e.to_string())?,
        }),
        ObjectiveWords::Engage { contact } => {
            let contact = if let Some(wanted) = contact {
                let unit = op
                    .roster
                    .plan
                    .slots
                    .iter()
                    .find(|s| s.controller.character() == Some(ctx.actor))
                    .and_then(|s| s.unit)
                    .ok_or("Own field unit unavailable.")?;
                let holders = [
                    PrincipalId::Object { id: ctx.actor },
                    PrincipalId::Object { id: unit },
                ];
                let wanted = wanted.to_ascii_lowercase();
                let contacts = ctx
                    .w
                    .module_state::<mush_core::ContactLedgerState>()
                    .into_iter()
                    .flat_map(|l| l.rows())
                    .filter(|r| {
                        r.active
                            && holders.contains(&r.holder)
                            && !wanted.is_empty()
                            && r.contact.to_hex().starts_with(&wanted)
                    })
                    .map(|r| r.contact)
                    .collect::<std::collections::BTreeSet<_>>();
                if contacts.len() != 1 {
                    return Err("No such contact in your own observation.".into());
                }
                contacts.into_iter().next()
            } else {
                None
            };
            Some(FieldObjective::Engage { contact })
        }
    };
    let directive = objective.map_or(Directive::Cancel, |objective| {
        Directive::Objective(FieldPayload {
            objective,
            hold_fire: words.hold_fire,
        })
    });
    submit(
        ctx,
        h,
        op,
        Act::Issue {
            revision,
            term,
            target,
            directive,
            expires_at_ms,
        },
    )
}
pub(crate) fn field_response(
    ctx: &mut Ctx<'_>,
    raw: &str,
    response: Response,
) -> Option<Result<(), String>> {
    let (fence, sequence) = raw.split_once(char::is_whitespace)?;
    let [detachment, revision] = crate::detachment::commands::numbers(fence).ok()?;
    let (h, op) = own(ctx, detachment, revision)?;
    let sequence = match sequence.trim().parse::<u64>() {
        Ok(n) if n > 0 => n,
        _ => return Some(Err("Invalid order sequence.".into())),
    };
    let slot = op
        .roster
        .plan
        .slots
        .iter()
        .find(|s| s.controller.character() == Some(ctx.actor))
        .map(|s| s.slot)?;
    Some(submit(
        ctx,
        &h,
        &op,
        Act::Respond {
            slot,
            sequence,
            response,
        },
    ))
}
fn line(ctx: &mut Ctx<'_>, text: String) {
    ctx.out.sys_to_actor(ctx.actor, "tacsim.detachment", text)
}
fn directive_text(directive: &Directive<FieldPayload>) -> String {
    let Directive::Objective(payload) = directive else {
        return "RELEASE".into();
    };
    let mut text = match &payload.objective {
        FieldObjective::Hold => "HOLD".into(),
        FieldObjective::Advance { to: (x, y) } => format!("ADVANCE {x},{y}"),
        FieldObjective::Engage { contact } => contact
            .as_ref()
            .map_or_else(|| "ENGAGE".into(), |id| format!("ENGAGE {}", id.to_hex())),
        FieldObjective::Withdraw => "WITHDRAW".into(),
        FieldObjective::Screen { bearing_mdeg } => format!("SCREEN {}", bearing_mdeg / 1000),
        FieldObjective::Support { slot } => format!("SUPPORT {}", slot.get()),
        FieldObjective::Mine { family } => {
            family.map_or_else(|| "MINE".into(), |family| format!("MINE {}", family.id()))
        }
    };
    if payload.hold_fire {
        text.push_str(" HOLD FIRE");
    }
    text
}
pub(super) fn report(ctx: &mut Ctx<'_>, op: &OperationRecord) -> Result<(), String> {
    use mush_propagation::{EnvelopeState, record::DispatchKind};
    let saved = op
        .command
        .as_deref()
        .ok_or("Refresh field command first.")?;
    let roster =
        mush_tactics::detachment::DetachmentRoster::restore(saved).map_err(|e| e.to_string())?;
    line(
        ctx,
        format!(
            "Field command term {}. Order prefill: ORDER DT:{}:{}:{} ALL HOLD",
            roster.term(),
            roster.id().0,
            roster.revision(),
            roster.term()
        ),
    );
    let Some(orders) = &op.orders else {
        line(ctx, "No field messages recorded.".into());
        return Ok(());
    };
    let book = OrderBook::restore(&orders.book, &roster).map_err(|e| e.to_string())?;
    // Issuers read their actual outbox, independently of the recipient's
    // command book. Only an arrived return packet discloses an acknowledgement.
    let outbox = if let Some(store) = ctx.w.module_state::<mush_propagation::PropagationStore>() {
        let prefix = format!(
            "dt:{}:{}:{}:",
            op.id.organizer.0, op.id.draft.0, op.id.activation
        );
        let mut sent = store
            .envelopes()
            .filter(|e| {
                e.kind() == DispatchKind::Detachment
                    && e.issuer() == ctx.actor
                    && e.dispatch_ref().is_some_and(|r| r.key.starts_with(&prefix))
            })
            .collect::<Vec<_>>();
        sent.sort_unstable_by_key(|e| std::cmp::Reverse(e.id()));
        sent.truncate(16);
        sent.reverse();
        sent.into_iter()
            .map(|e| {
                let key = e.dispatch_ref().map(|r| r.key.as_str()).unwrap_or("");
                let reply_prefix = format!("{key}:reply:");
                let reply = store
                    .envelopes()
                    .filter(|r| {
                        r.kind() == DispatchKind::DetachmentReply
                            && r.issuer() == e.recipient()
                            && r.recipient() == ctx.actor
                            && matches!(
                                r.state(),
                                EnvelopeState::Delivered | EnvelopeState::Acknowledged
                            )
                            && r.dispatch_ref()
                                .is_some_and(|d| d.key.starts_with(&reply_prefix))
                    })
                    .max_by_key(|r| r.id());
                let status = reply
                    .and_then(|r| r.dispatch_ref())
                    .and_then(|r| r.key.strip_prefix(&reply_prefix))
                    .map_or_else(
                        || "submitted; no reply confirmed".into(),
                        |r| format!("{} received", r.to_ascii_uppercase()),
                    );
                let (sequence, slot) = key
                    .strip_prefix(&prefix)
                    .and_then(|s| s.split_once(':'))
                    .unwrap_or(("?", "?"));
                format!("Sent slot {slot} order {sequence}: {status}. Execution unconfirmed.")
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    for message in outbox {
        line(ctx, message);
    }
    for member in roster.members() {
        let Some(delivered) = book.delivered(member.slot) else {
            continue;
        };
        let own = member.controller.character() == Some(ctx.actor);
        if own {
            line(
                ctx,
                format!(
                    "Slot {} received order {}: {}.",
                    member.slot.get(),
                    delivered.order.sequence,
                    directive_text(&delivered.order.directive)
                ),
            );
            line(
                ctx,
                format!(
                    "WILCO DT:{}:{} {} | NEGATIVE DT:{}:{} {}",
                    roster.id().0,
                    roster.revision(),
                    delivered.order.sequence,
                    roster.id().0,
                    roster.revision(),
                    delivered.order.sequence
                ),
            );
        }
    }
    Ok(())
}

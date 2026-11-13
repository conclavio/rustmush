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
    // Doc 45 unit AP.4a (Q-45-26): `<slot> POSTURE <word>`, the standing word of a roster
    // member's unit. An order like any other (the leader's authority, the relay, a jam), for a
    // player's slot only, and never from an OOC room (Q-45-19).
    let mut posture = text.split_whitespace();
    if let (Some(target), Some(verb)) = (posture.next(), posture.next())
        && verb.eq_ignore_ascii_case("POSTURE")
    {
        use crate::posture::order::{
            POSTURE_OOC, POSTURE_ORDER_CREW, POSTURE_ORDER_USAGE, POSTURE_WORDS,
        };
        if crate::posture::order::ooc(ctx.w, ctx.actor) {
            return Err(POSTURE_OOC.into());
        }
        let slot = Slot::new(target.parse().map_err(|_| POSTURE_ORDER_USAGE)?)
            .map_err(|e| e.to_string())?;
        let word = posture
            .next()
            .and_then(crate::posture::Posture::parse)
            .ok_or(POSTURE_WORDS)?;
        if posture.next().is_some() {
            return Err(POSTURE_ORDER_USAGE.into());
        }
        let roster = op
            .command
            .as_deref()
            .and_then(|saved| mush_tactics::detachment::DetachmentRoster::restore(saved).ok())
            .ok_or("Refresh field command first.")?;
        match roster.member(slot).map(|member| member.controller) {
            Some(mush_tactics::detachment::Controller::Player { .. }) => {}
            Some(_) => return Err(POSTURE_ORDER_CREW.into()),
            None => return Err("No such slot in this detachment.".into()),
        }
        return submit(
            ctx,
            h,
            op,
            Act::Issue {
                revision,
                term,
                target: OrderTarget::Slot(slot),
                directive: Directive::Objective(FieldPayload {
                    objective: FieldObjective::Posture { posture: word },
                    hold_fire: false,
                }),
                expires_at_ms,
            },
        );
    }
    // Doc 44 unit MW.14c: `<slot|ALL> CONCENTRATE <contact>`, the focus-fire order. An order
    // like any other (the leader's authority, each unit's command right, the relay, a jam).
    // `ALL` reaches every hull that can take it now and is not refused for one that cannot.
    let mut concentrate = text.split_whitespace();
    if let (Some(target), Some(verb)) = (concentrate.next(), concentrate.next())
        && verb.eq_ignore_ascii_case("CONCENTRATE")
    {
        let target = if target.eq_ignore_ascii_case("ALL") {
            OrderTarget::Able
        } else {
            OrderTarget::Slot(
                Slot::new(target.parse().map_err(|_| super::CONCENTRATE_USAGE)?)
                    .map_err(|e| e.to_string())?,
            )
        };
        let (Some(contact), None) = (concentrate.next(), concentrate.next()) else {
            return Err(super::CONCENTRATE_USAGE.into());
        };
        if expires_at_ms.is_some() {
            return Err(super::CONCENTRATE_NO_EXPIRY.into());
        }
        let objective = concentrate_on(ctx, op, contact)?;
        return submit(
            ctx,
            h,
            op,
            Act::Issue {
                revision,
                term,
                target,
                directive: Directive::Objective(FieldPayload {
                    objective,
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
            if let Some(wanted) = contact {
                engage_at(ctx, op, &wanted)?
            } else {
                Some(FieldObjective::Engage {
                    contact: None,
                    at: None,
                })
            }
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
/// **What a `CONCENTRATE <word>` names** (doc 44 unit MW.14c): a contact of the leader's own
/// cockpit inside a running field, by its designator, which the leader has identified. A
/// contact still under a provisional number is named differently on every scope, so no other
/// hull could find it in its own observation; the leader is told to identify it first.
fn concentrate_on(
    ctx: &Ctx<'_>,
    op: &OperationRecord,
    wanted: &str,
) -> Result<FieldObjective, String> {
    let unit = op
        .roster
        .plan
        .slots
        .iter()
        .find(|s| s.controller.character() == Some(ctx.actor))
        .and_then(|s| s.unit)
        .ok_or("Own field unit unavailable.")?;
    match crate::field::orders::leader_contact(ctx.w, ctx.actor, unit, wanted) {
        Some(field_orders::EngageAt::Contact {
            ordinal,
            id,
            designator,
        }) => {
            if !crate::field::orders::identified(ctx.w, ctx.actor, unit, id) {
                return Err(super::CONCENTRATE_UNIDENTIFIED.into());
            }
            Ok(FieldObjective::Concentrate {
                ordinal,
                id,
                designator,
            })
        }
        _ => Err(super::CONCENTRATE_NO_CONTACT.into()),
    }
}
/// **What an `ENGAGE <word>` names** (doc 32 cut CF.6): inside a field a contact of the leader's
/// own cockpit by its designator; else a campaign contact by its key's prefix; else, on open
/// ground, a flight on the presence picture (`NE`, `NE-near`).
fn engage_at(
    ctx: &Ctx<'_>,
    op: &OperationRecord,
    wanted: &str,
) -> Result<Option<FieldObjective>, String> {
    let unit = op
        .roster
        .plan
        .slots
        .iter()
        .find(|s| s.controller.character() == Some(ctx.actor))
        .and_then(|s| s.unit)
        .ok_or("Own field unit unavailable.")?;
    let engage = |contact, at| Ok(Some(FieldObjective::Engage { contact, at }));
    if let Some(at) = crate::field::orders::leader_contact(ctx.w, ctx.actor, unit, wanted) {
        return engage(None, Some(at));
    }
    let holders = [
        PrincipalId::Object { id: ctx.actor },
        PrincipalId::Object { id: unit },
    ];
    let folded = wanted.to_ascii_lowercase();
    let contacts = ctx
        .w
        .module_state::<mush_core::ContactLedgerState>()
        .into_iter()
        .flat_map(|l| l.rows())
        .filter(|r| {
            r.active
                && holders.contains(&r.holder)
                && !folded.is_empty()
                && r.contact.to_hex().starts_with(&folded)
        })
        .map(|r| r.contact)
        .collect::<std::collections::BTreeSet<_>>();
    if contacts.len() == 1 {
        return engage(contacts.into_iter().next(), None);
    }
    match crate::field::orders::row_of_words(wanted) {
        Some(at) => engage(None, Some(at)),
        None => Err("No such contact in your own observation.".into()),
    }
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
    // Doc 45 unit AP.4a: a POSTURE is set on the unit when it arrives and answered by the
    // unit. Its player's own WILCO or NEGATIVE is not taken: an answer ahead of the host's
    // would be read as the order already carried out, and a NEGATIVE would be a veto the
    // ruling does not give (the player sets the posture back instead).
    if posture_at(&op, slot, sequence) {
        return Some(Err(crate::posture::order::POSTURE_ORDER_ANSWERED.into()));
    }
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
/// Whether the order delivered to `slot` under `sequence` is a `POSTURE` (doc 45 unit AP.4a).
fn posture_at(op: &OperationRecord, slot: Slot, sequence: u64) -> bool {
    let Some(roster) = op
        .command
        .as_deref()
        .and_then(|saved| mush_tactics::detachment::DetachmentRoster::restore(saved).ok())
    else {
        return false;
    };
    op.orders
        .as_deref()
        .and_then(|orders| OrderBook::restore(&orders.book, &roster).ok())
        .and_then(|book| book.delivered(slot).cloned())
        .is_some_and(|delivered| {
            delivered.order.sequence == sequence
                && matches!(
                    delivered.order.directive,
                    Directive::Objective(FieldPayload {
                        objective: FieldObjective::Posture { .. },
                        ..
                    })
                )
        })
}
/// **The orders this roster's NPC leaders gave, as `+sortie/orders` shows them** (doc 44 unit
/// MW.14c): a head line, then one line an order, oldest first, each within 120 columns.
fn npc_orders(
    ctx: &Ctx<'_>,
    op: &OperationRecord,
    orders: &field_orders::FieldOrders,
) -> Vec<String> {
    use mush_propagation::{EnvelopeState, record::DispatchKind};
    if orders.given.is_empty() {
        return Vec::new();
    }
    let store = ctx.w.module_state::<mush_propagation::PropagationStore>();
    let mut lines = vec![super::NPC_ORDERS_HEAD.to_owned()];
    for given in &orders.given {
        let key = field_orders::message_key(op.id, given.sequence, given.slot);
        let reply_prefix = format!("{key}:reply:");
        let answer = store.and_then(|store| {
            store
                .envelopes()
                .filter(|reply| {
                    reply.kind() == DispatchKind::DetachmentReply
                        && reply.recipient() == given.issuer
                        && matches!(
                            reply.state(),
                            EnvelopeState::Delivered | EnvelopeState::Acknowledged
                        )
                        && reply
                            .dispatch_ref()
                            .is_some_and(|d| d.key.starts_with(&reply_prefix))
                })
                .max_by_key(|reply| reply.id())
                .and_then(|reply| reply.dispatch_ref())
                .and_then(|reply| reply.key.strip_prefix(&reply_prefix))
                .map(str::to_ascii_uppercase)
        });
        let status = match answer {
            Some(word) => format!("{word} received"),
            None => "no reply confirmed".to_owned(),
        };
        let name = ctx.w.get(given.issuer).map_or_else(
            || format!("#{}", given.issuer.0),
            |object| object.name.to_string(),
        );
        lines.push(npc_order_line(
            &name,
            (given.slot.get(), given.sequence),
            &directive_text(&given.directive),
            &status,
        ));
    }
    lines
}
/// **One of an NPC leader's orders** ([`super::NPC_ORDER`]), cut to 120 columns: the leader's
/// name gives way first, then the order's words.
fn npc_order_line(name: &str, (slot, sequence): (u16, u64), order: &str, status: &str) -> String {
    let fill = |name: &str, order: &str| {
        super::NPC_ORDER
            .replace("{leader}", name)
            .replace("{slot}", &slot.to_string())
            .replace("{sequence}", &sequence.to_string())
            .replace("{order}", order)
            .replace("{status}", status)
    };
    let cut = |text: &str, keep: usize| -> String {
        if text.chars().count() <= keep {
            text.to_owned()
        } else {
            let mut short: String = text.chars().take(keep.saturating_sub(1)).collect();
            short.push('~');
            short
        }
    };
    let whole = fill(name, order);
    if whole.chars().count() <= 120 {
        return whole;
    }
    let name = cut(name, 24);
    let fixed = fill(&name, "").chars().count();
    fill(&name, &cut(order, 120usize.saturating_sub(fixed).max(8)))
}
fn line(ctx: &mut Ctx<'_>, text: String) {
    ctx.out.sys_to_actor(ctx.actor, "tacsim.detachment", text)
}
pub(super) fn directive_text(directive: &Directive<FieldPayload>) -> String {
    let Directive::Objective(payload) = directive else {
        return "RELEASE".into();
    };
    let mut text = match &payload.objective {
        FieldObjective::Hold => "HOLD".into(),
        FieldObjective::Advance { to: (x, y) } => format!("ADVANCE {x},{y}"),
        FieldObjective::Engage { contact, at } => match (contact, at) {
            (Some(id), _) => format!("ENGAGE {}", id.to_hex()),
            (None, Some(field_orders::EngageAt::Contact { designator, .. })) => {
                format!("ENGAGE {designator}")
            }
            (None, Some(field_orders::EngageAt::Row { bearing, band })) => match band {
                Some(band) => format!("ENGAGE {bearing}-{band}"),
                None => format!("ENGAGE {bearing}"),
            },
            (None, None) => "ENGAGE".into(),
        },
        FieldObjective::Withdraw => "WITHDRAW".into(),
        FieldObjective::Screen { bearing_mdeg } => format!("SCREEN {}", bearing_mdeg / 1000),
        FieldObjective::Support { slot } => format!("SUPPORT {}", slot.get()),
        FieldObjective::Mine { family } => {
            family.map_or_else(|| "MINE".into(), |family| format!("MINE {}", family.id()))
        }
        FieldObjective::Posture { posture } => format!("POSTURE {}", posture.label()),
        FieldObjective::Concentrate { designator, .. } => format!("CONCENTRATE {designator}"),
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
        super::ORDER_PREFILL
            .replace("{term}", &roster.term().to_string())
            .replace("{roster}", &roster.id().0.to_string())
            .replace("{revision}", &roster.revision().to_string()),
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
                // Doc 45 unit AP.8: an order that lapsed while its leader was away says so, in
                // a sentence of its own after the line. The line itself is a constant a player
                // client parses for the reply's word (`ORDER_SENT`), and reads as it always did.
                let lapsed = sequence
                    .parse::<u64>()
                    .ok()
                    .zip(slot.parse::<u16>().ok().and_then(|n| Slot::new(n).ok()))
                    .is_some_and(|(sequence, slot)| orders.lapse_of(slot, sequence).is_some());
                let sent = super::ORDER_SENT
                    .replace("{slot}", slot)
                    .replace("{sequence}", sequence)
                    .replace("{status}", &status);
                if lapsed {
                    format!("{sent} {}", super::ORDER_LAPSED_STATUS)
                } else {
                    sent
                }
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    for message in outbox {
        line(ctx, message);
    }
    // Doc 44 unit MW.14c: what the roster's NPC leader ordered, and how each was answered.
    // The organiser and every player flying a slot may read it (`super::commands::read`
    // refuses anyone else the whole report); the NPC is theirs, hired under the organiser's
    // contract, and leads their units. As with a player's own outbox, an answer is shown only
    // once its reply has reached the leader who gave the order.
    for message in npc_orders(ctx, op, orders) {
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
            // Doc 45 unit AP.4a: a POSTURE is the unit's to answer; no prefill is offered.
            if matches!(
                delivered.order.directive,
                Directive::Objective(FieldPayload {
                    objective: FieldObjective::Posture { .. },
                    ..
                })
            ) {
                continue;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// **Doc 44 unit MW.14c**: each line `+sortie/orders` prints of an NPC leader's orders
    /// fits 120 columns, whatever the leader is called and whatever the order says, and a
    /// line that fits is printed whole.
    #[test]
    fn mw14c_an_npc_leaders_order_line_fits_120_columns() {
        let whole = npc_order_line("Sana Okonkwo", (2, 7), "ADVANCE 128,118", "WILCO received");
        assert_eq!(
            whole,
            "Sana Okonkwo to slot 2, order 7: ADVANCE 128,118; WILCO received."
        );
        let name = "A".repeat(64);
        let order = format!("ENGAGE {} HOLD FIRE", "f".repeat(64));
        for status in ["no reply confirmed", "NEGATIVE received", "WILCO received"] {
            let line = npc_order_line(&name, (6, u64::MAX), &order, status);
            assert!(line.chars().count() <= 120, "{line}");
            assert!(line.ends_with(&format!("; {status}.")), "{line}");
            assert!(line.contains("to slot 6, order 18446744073709551615: ENGAGE"));
        }
        assert!(super::super::NPC_ORDERS_HEAD.chars().count() <= 120);
        for line in [
            super::super::CONCENTRATE_USAGE,
            super::super::CONCENTRATE_UNIDENTIFIED,
            super::super::CONCENTRATE_NO_CONTACT,
            super::super::CONCENTRATE_NO_EXPIRY,
        ] {
            assert!(line.chars().count() <= 120, "{line}");
        }
        assert_eq!(
            directive_text(&Directive::Objective(FieldPayload {
                objective: FieldObjective::Concentrate {
                    ordinal: 1,
                    id: 70,
                    designator: smol_str::SmolStr::new("BK"),
                },
                hold_fire: false,
            })),
            "CONCENTRATE BK"
        );
    }
}

//! **The surface tick's claim arm** (doc 23 cut S.4b-2; §17.3, D-23-13, D-23-21, D-23-23).
//!
//! A rig whose `RigRecord.claim` is owed a transition is the arm's: an `Opening` cursor (the
//! rig in its bay, the launch queued behind it) or a `Closing` one (a finish closed the epoch).
//! Each visit is one closed call **as the cursor's operator** through
//! [`crate::port_tick::Fire::as_principal`], the tick's one-act-per-principal rule:
//!
//! - **`Opening`, the book holding no active Open for the rig**: the arm mints the
//!   `StartPermit` from the record (`minds::ic_sortie::surface`), enqueues it once a boot on the
//!   IC host and submits its ticket;
//! - **`Opening`, the book holding the operator's active Open**: the Open is acknowledged out of
//!   the host's inbox and the queued launch's own start is submitted (`act: launch`, no `mind=`
//!   word, under the queued key), whose prepare moves the cursor to `Open` and starts the ramp in
//!   one bundle (`surface::act`'s `queued_claim`);
//! - **`Closing`, the book's row not closed**: the arm mints the `CloseEvent`, enqueues it once
//!   and submits; once the row is closed the Close is acknowledged and nothing more is owed (the
//!   next `go` replaces the cursor).
//!
//! **Nothing here is durable**: the tickets are RAM, keyed by rig, and a reboot re-owes every
//! transition from the record (the host's inbox is empty at boot by design). Each transition is
//! observed in the book, never by request id. The arm holds the composed host only where the
//! claim path is composed (`TacsimRuleset::ic_host`); elsewhere no record carries a cursor.
use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{Ctx, ObjId};

use crate::minds::ic_sortie::{ClaimRead, IcHost, IcHostTicket, claim_closed, claim_of};
use crate::port_tick::{Fire, Step, SwapRefused};
use crate::surface::excursion::{ClaimCloseKind, ClaimCursor, RigRecord};

/// **The arm's slot**: the composed IC host, and each rig's ticket in flight this boot.
#[derive(Clone, Default)]
pub(crate) struct ClaimArm {
    host: Option<IcHost>,
    opens: BTreeMap<ObjId, Arc<IcHostTicket>>,
    closes: BTreeMap<ObjId, Arc<IcHostTicket>>,
}

/// Register the arm's slot with the composed host, if any.
pub(crate) fn register(reg: &mut mush_core::Registrar<'_>, host: Option<IcHost>) {
    reg.state(ClaimArm {
        host,
        ..ClaimArm::default()
    });
}

/// **Whether `record` is owed a claim transition** (the tick's `claim` candidate reason): an
/// `Opening` cursor with nothing in flight, or a `Closing` one whose Close the book does not
/// hold yet.
pub(crate) fn owed(w: &mush_core::World, record: &RigRecord) -> bool {
    match record.claim.as_deref() {
        Some(ClaimCursor::Opening { .. }) => record.act.is_none() && record.run.is_none(),
        Some(ClaimCursor::Closing { open, .. }) => claim_closed(w, *open).is_none(),
        _ => false,
    }
}

/// **One visit to a rig owed a claim transition**, as its operator. `start` is the surface act's
/// action (the queued launch's start), `ordinal` the ordinal it will take.
pub(crate) fn step(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    start: Option<&mush_core::DefinitionRef>,
    record: &RigRecord,
) -> Step {
    let tv = record.tv;
    let Some(mut arm) = ctx.w.module_state::<ClaimArm>().cloned() else {
        return Step::Refused;
    };
    let Some(host) = arm.host.clone() else {
        return Step::Refused;
    };
    let step = match record.claim.as_deref() {
        Some(ClaimCursor::Opening {
            operator,
            licence,
            launch_ic,
            ..
        }) => match claim_of(ctx.w, tv) {
            ClaimRead::Active { dispatcher, .. } if dispatcher == *operator => {
                if let Some(ticket) = arm.opens.remove(&tv) {
                    let _ = host.acknowledge(ctx.w, &ticket);
                }
                start.map_or(Step::Refused, |start| {
                    submit_launch(ctx, fire, start, record, *operator)
                })
            }
            ClaimRead::Active { .. } => Step::Refused,
            ClaimRead::None => {
                let ticket = match arm.opens.get(&tv) {
                    Some(ticket) => Arc::clone(ticket),
                    None => {
                        let minted = crate::minds::ic_sortie::surface::surface_start(
                            ctx.w, *operator, tv, *launch_ic,
                        )
                        .and_then(|permit| {
                            let licence = mush_roll::commerce::LicenceId::new(*licence)
                                .map_err(|_| refused("licence"))?;
                            host.enqueue_start(&permit, licence, vec![])
                        });
                        match minted {
                            Ok(ticket) => {
                                let ticket = Arc::new(ticket);
                                arm.opens.insert(tv, Arc::clone(&ticket));
                                ticket
                            }
                            Err(error) => {
                                tracing::warn!(tv = tv.0, %error, "tacsim: a rig's mind claim could not be enqueued; it waits");
                                return put(ctx, arm, Step::Deferred);
                            }
                        }
                    }
                };
                submit(ctx, fire, &host, &ticket, (tv, *operator))
            }
        },
        Some(ClaimCursor::Closing {
            open,
            kind,
            operator,
        }) => {
            if claim_closed(ctx.w, *open).is_some() {
                if let Some(ticket) = arm.closes.remove(&tv) {
                    let _ = host.acknowledge(ctx.w, &ticket);
                }
                Step::Idle
            } else {
                let ticket = match arm.closes.get(&tv) {
                    Some(ticket) => Arc::clone(ticket),
                    None => {
                        let minted = crate::minds::ic_sortie::surface::surface_close(
                            ctx.w,
                            *operator,
                            *open,
                            close_of(*kind),
                        )
                        .and_then(|event| host.enqueue_close(&event));
                        match minted {
                            Ok(ticket) => {
                                let ticket = Arc::new(ticket);
                                arm.closes.insert(tv, Arc::clone(&ticket));
                                ticket
                            }
                            Err(error) => {
                                tracing::warn!(tv = tv.0, %error, "tacsim: a rig's mind claim's close could not be enqueued; it waits");
                                return put(ctx, arm, Step::Deferred);
                            }
                        }
                    }
                };
                submit(ctx, fire, &host, &ticket, (tv, *operator))
            }
        }
        _ => Step::Idle,
    };
    put(ctx, arm, step)
}

fn put(ctx: &mut Ctx<'_>, arm: ClaimArm, step: Step) -> Step {
    if let Some(slot) = ctx.w.module_state_mut::<ClaimArm>() {
        *slot = arm;
    }
    step
}

fn refused(what: &str) -> mush_core::participant::ParticipantError {
    mush_core::participant::ParticipantError::refused(&format!("surface claim {what}"))
}

fn close_of(kind: ClaimCloseKind) -> crate::minds::ic_sortie::SurfaceClose {
    use crate::minds::ic_sortie::SurfaceClose;
    match kind {
        ClaimCloseKind::ReturnedToBase { base } => SurfaceClose::ReturnedToBase(base),
        ClaimCloseKind::PoweredDownInBase { base } => SurfaceClose::PoweredDownInBase(base),
        ClaimCloseKind::Destroyed => SurfaceClose::Destroyed,
        ClaimCloseKind::RecoveredToBase { base } => SurfaceClose::RecoveredToBase(base),
    }
}

/// The ticket's one submission, as the operator (the host refuses any other actor).
fn submit(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    host: &IcHost,
    ticket: &IcHostTicket,
    (tv, operator): (ObjId, ObjId),
) -> Step {
    match fire.as_principal(ctx, tv, operator, |ctx| host.submit_as_host(ctx, ticket)) {
        Ok(Ok(())) => Step::Submitted,
        Ok(Err(error)) => {
            tracing::warn!(operator = operator.0, %error, "tacsim: a rig's mind claim was not submitted; it waits");
            Step::Refused
        }
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
    }
}

/// **The claim refused for good** (doc 23 cut S.4b-2, D-23-21): the tick's tries spent with no
/// Open in the book, the queued launch's start is submitted with `claim: refused`, whose prepare
/// writes `Refused` and cancels the queue; the operator is told once, here.
pub(crate) fn refuse(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    start: Option<&mush_core::DefinitionRef>,
    record: &RigRecord,
) -> Step {
    let Some(ClaimCursor::Opening { operator, .. }) = record.claim.as_deref() else {
        return Step::Idle;
    };
    let Some(start) = start else {
        return Step::Refused;
    };
    let operator = *operator;
    let tv = record.tv;
    let ordinal = record.ordinal.saturating_add(1);
    let Some(key) = crate::surface::act::queued_key(tv, ordinal).and_then(|key| {
        mush_core::IdempotencyKeyText::parse_canonical(format!("{}/refused", key.as_str())).ok()
    }) else {
        return Step::Refused;
    };
    let parameters = serde_json::json!({
        "tv": tv.0.to_string(),
        "act": crate::surface::excursion::ActKind::Launch.word(),
        "ordinal": ordinal,
        "claim": "refused",
    });
    match fire.as_principal(ctx, tv, operator, |ctx| {
        ctx.submit_authoritative(
            start.clone(),
            mush_core::PerceivedRef::SelfActor,
            Vec::new(),
            parameters,
            key,
        );
    }) {
        Ok(()) => {
            crate::port_tick::notify_budget(
                ctx,
                operator,
                crate::surface::excursion::CLAIM_REFUSED,
                crate::port_tick::NoticeLevel::Trip,
            );
            Step::Submitted
        }
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
    }
}

/// **The queued launch's start** (`act: launch`, no `mind=` word) under the queued key, as the
/// operator: its prepare finds the claim open and starts the ramp.
fn submit_launch(
    ctx: &mut Ctx<'_>,
    fire: &mut Fire,
    start: &mush_core::DefinitionRef,
    record: &RigRecord,
    operator: ObjId,
) -> Step {
    let tv = record.tv;
    let ordinal = record.ordinal.saturating_add(1);
    let Some(key) = crate::surface::act::queued_key(tv, ordinal) else {
        return Step::Refused;
    };
    let parameters = serde_json::json!({
        "tv": tv.0.to_string(),
        "act": crate::surface::excursion::ActKind::Launch.word(),
        "ordinal": ordinal,
    });
    match fire.as_principal(ctx, tv, operator, |ctx| {
        ctx.submit_authoritative(
            start.clone(),
            mush_core::PerceivedRef::SelfActor,
            Vec::new(),
            parameters,
            key,
        );
    }) {
        Ok(()) => Step::Submitted,
        Err(SwapRefused::AlreadyActed | SwapRefused::Owed | SwapRefused::Pending) => Step::Deferred,
        Err(SwapRefused::God | SwapRefused::Gone) => Step::Refused,
    }
}

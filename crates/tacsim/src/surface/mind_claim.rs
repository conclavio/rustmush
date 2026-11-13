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
//! **Nothing here is durable**: the tickets are RAM, and a reboot re-owes every transition from
//! the record (the host's inbox is empty at boot by design). Each transition is observed in the
//! book, never by request id. The arm holds the composed host only where the claim path is
//! composed (`TacsimRuleset::ic_host`); elsewhere no record carries a cursor.
//!
//! **A ticket is one claim's, not the rig's** (doc 44 MW.22's follow-up). A rig flies many
//! claims in one boot, so each ticket is kept with what it was minted for: an Open with its
//! epoch's instant (`launch_ic`), a Close with the claim's open token. A ticket kept for another
//! claim is never submitted: it is acknowledged out of the host's inbox and a new one minted.
//! And a Close that has landed is acknowledged on one more visit ([`owed`]), which releases its
//! inbox row. Before this the tickets were keyed by rig alone and a landed Close was never
//! visited again, so a rig's second claim in a boot resubmitted the first claim's spent Close
//! for ever: the claim stayed open and its licence's seat held until a restart.
//!
//! **An Open's ticket is released when its cursor no longer asks it** (doc 44 MW.13's finding;
//! the ship's arm's `Owes::Stale`). The host keeps one Open pending an actor and drops it only
//! when the book holds its claim (`IcHost::acknowledge`), and a claim refused for good never
//! reaches the book. So the Open of a claim given up stayed in the host, and its operator's
//! next mind launch, on that rig or another, could not be enqueued until a restart. Every fire
//! now begins with [`release_stale`]: a ticket whose rig's cursor is no longer the `Opening` it
//! was minted for is acknowledged, and where the book holds no claim for it, withdrawn
//! (`IcHost::withdraw_surface_open`).
use std::collections::BTreeMap;
use std::sync::Arc;

use mush_core::{Ctx, ObjId};

use crate::minds::ic_sortie::{ClaimRead, IcHost, IcHostTicket, claim_closed, claim_of};
use crate::port_tick::{Fire, Step, SwapRefused};
use crate::surface::excursion::{ClaimCloseKind, ClaimCursor, RigRecord};

/// **The arm's slot**: the composed IC host, and each rig's ticket in flight this boot, with
/// the claim it is for (the module note).
#[derive(Clone, Default)]
pub(crate) struct ClaimArm {
    host: Option<IcHost>,
    /// A rig's Open in flight, with the epoch instant (`launch_ic`) its `go` fixed.
    opens: BTreeMap<ObjId, (i64, Arc<IcHostTicket>)>,
    /// A rig's Close in flight, with the open token of the claim it closes.
    closes: BTreeMap<ObjId, (mush_core::CommitToken, Arc<IcHostTicket>)>,
}

impl ClaimArm {
    /// The rig's Open ticket, if it was minted for the epoch `launch_ic`.
    fn open_for(&self, tv: ObjId, launch_ic: i64) -> Option<Arc<IcHostTicket>> {
        self.opens
            .get(&tv)
            .filter(|(at, _)| *at == launch_ic)
            .map(|(_, ticket)| Arc::clone(ticket))
    }

    /// The rig's Close ticket, if it was minted for the claim `open`.
    fn close_for(&self, tv: ObjId, open: mush_core::CommitToken) -> Option<Arc<IcHostTicket>> {
        self.closes
            .get(&tv)
            .filter(|(of, _)| *of == open)
            .map(|(_, ticket)| Arc::clone(ticket))
    }
}

/// **The composed IC host**, where the claim path is composed (doc 23 cut S.4b-3: the field's
/// retained install reads it).
pub(crate) fn host_of(w: &mush_core::World) -> Option<IcHost> {
    w.module_state::<ClaimArm>()
        .and_then(|arm| arm.host.clone())
}

/// Register the arm's slot with the composed host, if any.
pub(crate) fn register(reg: &mut mush_core::Registrar<'_>, host: Option<IcHost>) {
    reg.state(ClaimArm {
        host,
        ..ClaimArm::default()
    });
}

/// **Release an Open's ticket the arm no longer needs**: acknowledged when the book holds its
/// claim (the host drops it), withdrawn when it does not (`IcHost::withdraw_surface_open`).
fn release(w: &mush_core::World, host: &IcHost, tv: ObjId, ticket: &IcHostTicket) {
    if matches!(host.acknowledge(w, ticket), Ok(Some(_))) {
        return;
    }
    if let Err(error) = host.withdraw_surface_open(w, ticket) {
        tracing::warn!(tv = tv.0, %error, "tacsim: a rig's mind claim's ticket was not released; its operator's next mind launch waits for a restart");
    }
}

/// **Release every Open's ticket its rig's cursor no longer asks** (the module note), before
/// the fire visits any claim. `records` is the fire's scan of the rig family and `complete`
/// whether it read all of it: a ticket whose rig has no record in a capped scan is left for a
/// later fire. A cursor still `Opening` at the ticket's instant keeps its ticket (its Open may
/// be on its way, and the arm resubmits it); any other cursor has given the Open up (refused
/// for good, cancelled, or replaced by a later `go`) or has seen it land.
pub(crate) fn release_stale(w: &mut mush_core::World, records: &[RigRecord], complete: bool) {
    let Some(arm) = w.module_state::<ClaimArm>() else {
        return;
    };
    if arm.opens.is_empty() {
        return;
    }
    let Some(host) = arm.host.clone() else {
        return;
    };
    let stale: Vec<ObjId> = arm
        .opens
        .iter()
        .filter(
            |(tv, (mark, _))| match records.iter().find(|record| record.tv == **tv) {
                Some(record) => !matches!(
                    record.claim.as_deref(),
                    Some(ClaimCursor::Opening { launch_ic, .. }) if launch_ic == mark
                ),
                None => complete,
            },
        )
        .map(|(tv, _)| *tv)
        .collect();
    if stale.is_empty() {
        return;
    }
    let Some(arm) = w.module_state_mut::<ClaimArm>() else {
        return;
    };
    let tickets: Vec<(ObjId, Arc<IcHostTicket>)> = stale
        .into_iter()
        .filter_map(|tv| arm.opens.remove(&tv).map(|(_, ticket)| (tv, ticket)))
        .collect();
    for (tv, ticket) in tickets {
        release(w, &host, tv, &ticket);
    }
}

/// **Whether `record` is owed a claim transition** (the tick's `claim` candidate reason): an
/// `Opening` cursor with nothing in flight, or a `Closing` one whose Close the book does not
/// hold yet, or holds while the arm still keeps its ticket (one more visit acknowledges the
/// landed Close out of the host's inbox, so the rig's next claim starts with none).
pub(crate) fn owed(w: &mush_core::World, record: &RigRecord) -> bool {
    match record.claim.as_deref() {
        Some(ClaimCursor::Opening { .. }) => record.act.is_none() && record.run.is_none(),
        Some(ClaimCursor::Closing { open, .. }) => {
            claim_closed(w, *open).is_none()
                || w.module_state::<ClaimArm>()
                    .is_some_and(|arm| arm.closes.contains_key(&record.tv))
        }
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
                if let Some((_, ticket)) = arm.opens.remove(&tv) {
                    let _ = host.acknowledge(ctx.w, &ticket);
                }
                // Doc 38a F-23.4: a staff `close` or `home` cleared the queued launch under the
                // opening claim; the claim opened anyway, so it is cancelled
                // (`PoweredDownInBase`), never left for a launch that will not come.
                if queued_launch(record) {
                    start.map_or(Step::Refused, |start| {
                        submit_launch(ctx, fire, start, record, *operator)
                    })
                } else {
                    refuse(ctx, fire, start, record)
                }
            }
            ClaimRead::Active { .. } => Step::Refused,
            // Doc 38a F-23.4: no launch is queued and no Open is in flight: the claim is refused,
            // never opened only to close.
            ClaimRead::None if !queued_launch(record) && arm.open_for(tv, *launch_ic).is_none() => {
                refuse(ctx, fire, start, record)
            }
            ClaimRead::None => {
                let ticket = match arm.open_for(tv, *launch_ic) {
                    Some(ticket) => ticket,
                    None => {
                        // A ticket kept for an earlier launch of this rig is not this claim's
                        // (`release_stale` has it first; this is the same rule where it is
                        // needed, were a fire's scan ever capped).
                        if let Some((_, stale)) = arm.opens.remove(&tv) {
                            release(ctx.w, &host, tv, &stale);
                        }
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
                                arm.opens.insert(tv, (*launch_ic, Arc::clone(&ticket)));
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
                if let Some((_, ticket)) = arm.closes.remove(&tv) {
                    let _ = host.acknowledge(ctx.w, &ticket);
                }
                Step::Idle
            } else {
                let ticket = match arm.close_for(tv, *open) {
                    Some(ticket) => ticket,
                    None => {
                        // A ticket kept for an earlier claim of this rig (its Close landed and
                        // was never acknowledged) is not this claim's.
                        if let Some((_, stale)) = arm.closes.remove(&tv) {
                            let _ = host.acknowledge(ctx.w, &stale);
                        }
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
                                arm.closes.insert(tv, (*open, Arc::clone(&ticket)));
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

/// Whether `record` still queues the launch its opening claim was minted for.
fn queued_launch(record: &RigRecord) -> bool {
    record
        .queued
        .as_ref()
        .is_some_and(|queued| queued.launch.is_some())
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

/// **The claim refused for good, or its launch cancelled** (doc 23 cuts S.4b-2 and S.4b-3,
/// D-23-21, D-23-23 step 5): the tick's tries spent, the queued launch's start is submitted with
/// `claim: refused`. With no Open in the book its prepare writes `Refused`; with the claim open
/// but the launch never started (refused at every try: the charge, the caps), it writes
/// `Closing { PoweredDownInBase }` and the arm closes the claim. Either way the queue is
/// cancelled and the operator is told once, here.
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
            // An open claim whose launch never ran is cancelled (it closes `PoweredDownInBase`);
            // a claim never opened is refused.
            let line = match claim_of(ctx.w, tv) {
                ClaimRead::Active { .. } => crate::surface::excursion::CLAIM_CANCELLED,
                ClaimRead::None => crate::surface::excursion::CLAIM_REFUSED,
            };
            crate::port_tick::notify_budget(
                ctx,
                operator,
                line,
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

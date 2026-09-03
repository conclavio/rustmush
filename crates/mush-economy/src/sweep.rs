//! The economy sweep (E4b1, 08c §4.3, D-E4-3) — the module's ONE tick.
//!
//! One `TickCadence::Coarse(SWEEP_PERIOD_SECS)` handler does everything time-driven
//! the economy has: it advances the cadence clocks, ages expired reservations out,
//! settles due obligation periods, and garbage-collects the dead records. There are
//! deliberately **no per-record timers** — the core scheduler is in-RAM and re-armed
//! from scratch at boot (so a tick count does not survive a restart), which is why
//! every catch-up here is computed from `next_due` and the record's own clock.
//!
//! ## The rule that shapes every line of this file
//!
//! A tick handler is **infallible from dispatch's point of view** (`TickHandler::tick`
//! returns `()`), and it runs inside the world task. There is no `Err` to truncate
//! partial work and no supervisor to restart anything: **a panic here aborts the
//! world**. So this module contains
//!
//! - no `?`, no `unwrap`, no `expect`, and no indexing that can panic;
//! - an **exhaustive** `match` on every outcome — [`disposition`] matches every
//!   [`EconomyError`] variant by name with no `_` arm, so a new error added later
//!   cannot silently fall into "retry forever";
//! - checked or saturating arithmetic throughout;
//! - hard caps on every loop (§9), so one fire is bounded whatever the world holds.
//!
//! ## The ladder, applied (§4.6, D-E4-14)
//!
//! - **Level 0 — resolve.** More periods due than `maximum_catch_up` are *forgiven*
//!   with a classified cause and an attributed history record. Doc 08 §2 forbids
//!   charging a downtime backlog, so this is the designed outcome, not a fallback.
//! - **Level 1 — inform.** A transient refusal (the journal is briefly unavailable)
//!   is logged and retried next fire; nothing stops.
//! - **Level 2 — page.** A refusal only a human can fix suspends **that obligation
//!   alone** and opens an [`Escalation`] carrying the command that fixes it.
//! - **Level 3 — stop.** The whole-economy freeze is the only world-stop; while it is
//!   set every step here is a no-op, and one companion record says so.
//!
//! Output is exception-driven (§14.3, D10): a routine settlement is **silent** unless
//! the obligation asked otherwise, and **no notice ever claims money durably moved**
//! (D-E4-6) — the sweep does not use the whole-step durable gate, so it has no
//! standing to make that claim.

use mush_core::{Ctx, SceneId};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::api::{self, PostAuth, PostOutcome};
use crate::clock::{CadenceSource, Clocks, advance_cadence};
use crate::engine::AuthEval;
use crate::escalation::{
    ESCALATION_GC_AFTER_SECS, Escalation, EscalationKind, EscalationSubject,
};
use crate::ids::{AccountId, EscalationId, HoldId, ObligationId};
use crate::module::EconomyStore;
use crate::obligation::{
    ForgiveCause, Obligation, ObligationEvent, ObligationStatus, SettleOutcome, SuspendReason,
    advance as advance_obligation, classify_forgiveness, forgive, forgive_all, obligation_key,
    plan_sweep,
};
use crate::plan::{EconomyError, EconomyPlan, HoldOp, RequestedLine};
use crate::record::{Account, AccountStatus, ApprovalStatus, EconomicCause, IdempotencyKey};

/// The registered tick tag.
pub const SWEEP_TAG: &str = "economy.sweep";
/// The most cadence clocks one fire advances (bounded like every other step).
const MAX_CADENCES_PER_SWEEP: usize = 64;
/// The most records of one kind one fire garbage-collects.
const MAX_GC_PER_SWEEP: usize = 64;
/// The most escalations one fire re-notifies about.
const MAX_RENOTIFY_PER_SWEEP: usize = 32;

/// One sweep fire (§4.3), in order: cadences → hold expiries → obligation settlement
/// → GC → escalation re-notification → the sweep stamps.
pub(crate) fn sweep(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    let now = ctx.w.now();
    let ic_now = ctx.w.game_now_unix();
    let ratio = i64::from(ctx.w.game_clock().ic_ratio.max(1));
    let Some(store) = ctx.w.module_state::<EconomyStore>() else {
        // No economy in this world: nothing to sweep, and nothing to complain about.
        return;
    };
    if store.frozen {
        // Level 3 (§4.6): the whole-economy freeze is the one world-stop. Every step
        // below is a no-op — including the sweep stamps, because a sweep that did no
        // work did not run, and stamping would let a long freeze classify as
        // `Overload` (which escalates on repeat) instead of `Downtime`.
        open_freeze_record(ctx, now);
        renotify(ctx, now);
        return;
    }
    let (last_real, last_ic) = store.last_sweep();
    advance_cadences(ctx, now, ic_now);
    expire_holds(ctx, now, ic_now);
    settle_obligations(ctx, now, ic_now, ratio, last_real, last_ic);
    collect_garbage(ctx, now);
    renotify(ctx, now);
    stamp_sweep(ctx, now, ic_now);
}

/// Persist the fire's two stamps — the evidence [`classify_forgiveness`] reads to
/// tell a restart from a starved post budget.
fn stamp_sweep(ctx: &mut Ctx<'_>, now: i64, ic_now: i64) {
    let meta = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        store.stamp_sweep(now, ic_now);
        store.meta_doc()
    };
    api::persist_meta(ctx.out, meta);
}

// ---- step 0: cadence clocks (D-E4-2) --------------------------------------

/// Advance every cadence clock from its own source, under its own catch-up cap.
///
/// [`advance_cadence`] returns a [`CadenceSource::Manual`] cadence **unchanged** (its
/// position is a human's to move, through `@obligation/advance` and E5's
/// `@econ/cadence`), so the filter below is belt to that suspenders: the sweep must
/// never advance a clock nobody asked it to advance.
fn advance_cadences(ctx: &mut Ctx<'_>, now: i64, ic_now: i64) {
    let moved: Vec<crate::clock::CadenceClock> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else { return };
        store
            .snapshot()
            .cadences()
            .filter(|c| c.source != CadenceSource::Manual)
            .take(MAX_CADENCES_PER_SWEEP)
            .filter_map(|c| {
                let source_now = match c.source {
                    CadenceSource::RealUtc => now,
                    CadenceSource::WorldIc => ic_now,
                    CadenceSource::Manual => return None,
                };
                let (next, forgiven) = advance_cadence(c, source_now);
                if forgiven > 0 {
                    tracing::info!(
                        cadence = c.id.0,
                        forgiven,
                        "economy: cadence catch-up cap reached; the excess periods are forgiven"
                    );
                }
                (next.position != c.position).then_some(next)
            })
            .collect()
    };
    for c in moved {
        api::put_cadence(ctx, c);
    }
}

// ---- step 1: hold expiries ------------------------------------------------

/// Age out every reservation whose deadline has passed **on its own clock**.
///
/// Effect-only posts with the deterministic key `hold/{id}/expire`, so two fires at
/// the same instant produce exactly one effect (the second resolves to
/// `AlreadyPosted`). A `Manual` deadline resolves to "never passed" here exactly as
/// it does in the engine: a manually-clocked hold is released by hand, never aged out
/// by a sweep reading someone else's clock.
fn expire_holds(ctx: &mut Ctx<'_>, now: i64, ic_now: i64) {
    let clocks = Clocks::new(now, ic_now);
    let due: Vec<HoldId> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else { return };
        let snap = store.snapshot();
        snap.holds()
            .filter(|h| h.status.is_held())
            .filter(|h| h.expires.is_some_and(|d| clocks.passed(&d, snap, i64::MIN)))
            // The engine refuses a hold op on a non-`Active` account (a frozen account
            // must not have its numbers moved even by an effect). Skipping here keeps
            // the sweep from posting a guaranteed refusal every 30 seconds; the hold
            // ages out as soon as the account is unfrozen.
            .filter(|h| {
                snap.account(h.account).is_some_and(|a| a.status == AccountStatus::Active)
            })
            .take(crate::MAX_HOLD_EXPIRIES_PER_SWEEP)
            .map(|h| h.id)
            .collect()
    };
    for id in due {
        let mut plan = EconomyPlan::new(
            IdempotencyKey::new(SmolStr::new_static("hold"), SmolStr::new(format!("{}/expire", id.0))),
            api::sweep_principal(),
            EconomicCause::Internal { kind: SmolStr::new_static("hold.expire"), id: id.0 },
            Vec::new(),
        );
        plan.hold_ops = vec![HoldOp::Expire { hold: id }];
        // Exhaustive, like every outcome in this file.
        match api::post_with(ctx, &plan, PostAuth::World) {
            Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => {}
            Ok(PostOutcome::NeedsApproval(request)) => {
                // Unreachable: an effect-only expiry has no net debit, so no
                // `ApprovalRule` can fire on it. Loud rather than silent.
                tracing::error!(hold = id.0, request = request.0, "economy: a hold expiry asked for approval");
            }
            Err(e) => {
                // A hold whose deadline passed but whose expiry is refused is not a
                // money problem: the reservation simply stands until the refusal is
                // fixed. Level 1 — inform, nothing stops.
                tracing::warn!(hold = id.0, error = %e, "economy: sweep could not expire a hold");
            }
        }
    }
}

// ---- step 2: obligations --------------------------------------------------

/// One obligation's narrow settlement authority.
///
/// The sweep is not staff and holds nobody's wallet rights: this authority permits
/// **exactly** the two lines the obligation itself declares, and nothing else. Every
/// other [`AuthEval`] method keeps its default `false`, so a settlement plan can move
/// no title, no custody, and no reservation.
///
/// A `System` debit is refused outright even when the obligation names one: debiting
/// a registered faucet creates money from nothing, which is staff-only, full stop
/// (the A6 rule). `@obligation/create` refuses that shape up front for the same
/// reason; this is the second lock.
struct ObligationAuth {
    debit: AccountId,
    credit: AccountId,
}

impl AuthEval for ObligationAuth {
    fn authorize(&self, _actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        if delta.is_negative() {
            return account.id == self.debit && !matches!(account.owner, PrincipalId::System(_));
        }
        account.id == self.credit
    }
}

/// What one period's post amounted to, from the sweep's point of view.
enum Disposition {
    /// The period is paid (or was already paid by an earlier fire).
    Settled(SettleOutcome),
    /// The debtor's **available** funds were short — the grace path, a story state.
    Short,
    /// Transient: retry on the next fire, change nothing (level 1).
    Transient,
    /// Only a human can fix this (level 2): suspend the subject and page, with the
    /// command filled in. The kind is what the alert is *called*.
    Human(EscalationKind),
}

/// Map an [`EconomyError`] onto the ladder — **exhaustively, by name**.
///
/// No `_` arm on purpose: a new refusal added to the engine must be classified here
/// deliberately, because the two failure modes this guards against are opposite and
/// both bad. Treating a human problem as transient retries a doomed post forever and
/// nobody is ever told; treating a transient problem as human suspends a healthy
/// obligation and pages a staffer at three in the morning.
fn disposition(e: &EconomyError) -> Disposition {
    match e {
        // The money is there but spoken for, or it is not there at all: a miss, which
        // is a story state (grace → delinquency), never a staff page.
        EconomyError::InsufficientAvailable(_) | EconomyError::CreditViolation(_) => {
            Disposition::Short
        }
        // Transient by construction — the next fire is the retry.
        EconomyError::JournalUnavailable | EconomyError::EconomyFrozen => Disposition::Transient,
        // A settlement refusal only a human can fix (§4.3): a closed/missing/frozen
        // account, a retired instrument, an inactive org compartment, an approval rule
        // the sweep cannot satisfy, or a deterministic-key conflict (doc 09 C8's
        // integrity incident, scoped to this obligation alone).
        EconomyError::NoSuchAccount(_)
        | EconomyError::InactiveAccount(_)
        | EconomyError::NoSuchInstrument(_)
        | EconomyError::InactiveInstrument(_)
        | EconomyError::Unauthorized
        | EconomyError::ScopeOwnerMismatch
        | EconomyError::ApprovalRequired
        | EconomyError::ApprovalExpired
        | EconomyError::Overflow => Disposition::Human(EscalationKind::SettlementRefused),
        // Doc 09 C8: a conflict on an internal deterministic key is an integrity
        // incident, and the response is to freeze the SMALLEST scope — this
        // obligation, through the ladder — rather than the economy.
        EconomyError::IdempotencyConflict => Disposition::Human(EscalationKind::KeyConflict),
        // Everything below cannot arise from a two-line obligation plan (it carries no
        // asset, custody, hold or contract op, no reversal, and no revision guard).
        // Reaching one means the settlement builder itself is wrong, which is exactly
        // the kind of thing that must surface as an alert rather than as silence.
        EconomyError::TooFewLines
        | EconomyError::ZeroLine(_)
        | EconomyError::NoSuchTransaction(_)
        | EconomyError::NotBalanced(_)
        | EconomyError::AlreadyReversed(_)
        | EconomyError::Stale { .. }
        | EconomyError::TooLarge(_)
        | EconomyError::NoSuchAsset(_)
        | EconomyError::AssetExists(_)
        | EconomyError::AssetNotActive(_)
        | EconomyError::AssetFrozen(_)
        | EconomyError::Encumbered(_)
        | EconomyError::NoSuchEncumbrance(_)
        | EconomyError::AssignmentToOwner(_)
        | EconomyError::NoSuchVault(_)
        | EconomyError::VaultNotActive(_)
        | EconomyError::NotInVault(_)
        | EconomyError::NoCustodyStanding(_)
        | EconomyError::BadDestination(_)
        | EconomyError::BackingNotDepositable(_)
        | EconomyError::BackingInvalid(_)
        | EconomyError::DuplicateAssetOp(_)
        | EconomyError::DuplicateCustodyOp(_)
        | EconomyError::HasEffects
        | EconomyError::Custody(_)
        | EconomyError::StaleAsset { .. }
        | EconomyError::NoSuchHold(_)
        | EconomyError::HoldAmountNotPositive
        | EconomyError::HoldNotHeld(_)
        | EconomyError::HoldOnUnboundedAccount(_)
        | EconomyError::CaptureExceedsHold(_)
        | EconomyError::HoldNotExpired(_)
        | EconomyError::HoldExpireNotSweep
        | EconomyError::DuplicateHoldOp(_)
        | EconomyError::CaptureUnbacked(_)
        | EconomyError::HoldIdReused(_)
        | EconomyError::StaleHold { .. }
        | EconomyError::CustodyLocked(_)
        | EconomyError::NoSuchContract(_)
        | EconomyError::ContractState(_)
        | EconomyError::ConsentMismatch(_)
        | EconomyError::NoSuchObligation(_) => Disposition::Human(EscalationKind::SettlementRefused),
    }
}

/// Settle the due pool, oldest `next_due` first, under both caps (§9): at most
/// [`crate::MAX_OBLIGATIONS_PER_SWEEP`] obligations and
/// [`crate::MAX_POSTS_PER_SWEEP`] settlement posts per fire. Work left over waits for
/// the next fire — and if an obligation is starved often enough, the forgiveness it
/// eventually takes is classified `Overload`, which escalates on repeat.
fn settle_obligations(
    ctx: &mut Ctx<'_>,
    now: i64,
    ic_now: i64,
    ratio: i64,
    last_real: i64,
    last_ic: i64,
) {
    let due: Vec<ObligationId> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else { return };
        let mut pool: Vec<(i64, ObligationId)> = store
            .snapshot()
            .obligations()
            // The due pool is `Active` + `Grace` ONLY (D-E4-3): that, not the per-fire
            // cap, is what stops a long-suspended obligation starving live ones.
            .filter(|o| o.status.in_due_pool())
            .map(|o| (o.schedule.next_due, o.id))
            .collect();
        pool.sort_unstable();
        pool.truncate(crate::MAX_OBLIGATIONS_PER_SWEEP);
        pool.into_iter().map(|(_, id)| id).collect()
    };
    let mut posts = 0usize;
    for id in due {
        if posts >= crate::MAX_POSTS_PER_SWEEP {
            break;
        }
        settle_one(ctx, id, now, ic_now, ratio, last_real, last_ic, &mut posts);
    }
}

/// One obligation's whole sweep step.
#[allow(clippy::too_many_arguments)]
fn settle_one(
    ctx: &mut Ctx<'_>,
    id: ObligationId,
    now: i64,
    ic_now: i64,
    ratio: i64,
    last_real: i64,
    last_ic: i64,
    posts: &mut usize,
) {
    let clocks = Clocks::new(now, ic_now);
    let Some((mut o, clock_now)) = ({
        let Some(store) = ctx.w.module_state::<EconomyStore>() else { return };
        let snap = store.snapshot();
        snap.obligation(id).and_then(|o| {
            // A schedule on a cadence whose record is missing resolves to `None`, and
            // every consumer treats that as **not yet**: a clock nobody can read never
            // makes a payment due.
            clocks.now_on(o.schedule.clock, snap, o.schedule.manual_now).map(|n| (o.clone(), n))
        })
    }) else {
        return;
    };
    let before = o.status.clone();
    let intent = plan_sweep(&o, clock_now);
    if intent.is_empty() {
        return;
    }

    // ---- level 0: the forgive half, BEFORE any attempt -------------------
    //
    // `plan_sweep`'s `attempt` list starts *past* the periods it asked to have
    // forgiven, so posting the attempts first would hand `advance` a period ahead of
    // `periods_settled` — which it refuses as `OutOfOrder` precisely to keep this
    // ordering honest (E4a verify fold, finding 9).
    let mut forgiven_here: Option<ForgiveCause> = None;
    if intent.forgive > 0 {
        let cause = classify_forgiveness(last_real, last_ic, now, ic_now, ratio);
        o = if intent.reanchor {
            // The period counter cannot advance without handing two periods one
            // idempotency key: forgive wholesale, consume no index, re-anchor.
            forgive_all(&o, intent.forgive, cause, clock_now)
        } else {
            forgive(&o, intent.forgive_from, intent.forgive_to, cause, clock_now)
        };
        forgiven_here = Some(cause);
        notify_both(
            ctx,
            &o,
            format!(
                "Obligation #{id}: {} period(s) were forgiven ({}). Nothing was charged for them.",
                intent.forgive,
                cause_word(cause)
            ),
        );
    }

    // `Overload` twice inside the window is a misconfigured cadence or a starved pool
    // — a question only a human can answer (§4.6). The window is measured on the
    // obligation's OWN clock, the same clock its history is stamped on.
    if forgiven_here == Some(ForgiveCause::Overload) && overload_repeats(&o, clock_now) {
        let record = page(
            ctx,
            EscalationKind::RepeatedOverload,
            EscalationSubject::Obligation(id),
            format!(
                "obligation #{id} has been forgiven for overload twice inside the escalation \
                 window: it keeps missing its post budget, which usually means its cadence is \
                 far shorter than the sweep period or the pool is starved"
            ),
            obligation_advice(&o, None),
            now,
        );
        if let Some(eid) = record {
            o = suspend_for(&o, eid, clock_now);
            notify_both(ctx, &o, format!("Obligation #{id} is suspended pending staff (alert #{eid})."));
        }
        api::put_obligation(ctx, o);
        return;
    }
    if intent.reanchor {
        api::put_obligation(ctx, o);
        return;
    }

    // ---- the attempts ----------------------------------------------------
    for period in intent.attempt {
        if *posts >= crate::MAX_POSTS_PER_SWEEP {
            break;
        }
        *posts += 1;
        let (disposition, detail, blamed) = post_period(ctx, &o, period);
        match disposition {
            Disposition::Settled(outcome) => match advance_obligation(&o, period, outcome, clock_now) {
                Ok(next) => o = next,
                Err(e) => {
                    // `OutOfOrder` (and a re-delivered duplicate, which `advance`
                    // absorbs) can only arise from a bug in THIS file's ordering, not
                    // from anything the world did. Loud, and never a panic: a panic
                    // here would take the world task down over a bookkeeping slip.
                    tracing::error!(obligation = id.0, period, error = %e, "economy: sweep folded a settlement outcome out of order (programming error)");
                    break;
                }
            },
            Disposition::Short => {
                match advance_obligation(&o, period, SettleOutcome::InsufficientAvailable, clock_now) {
                    Ok(next) => o = next,
                    Err(e) => {
                        tracing::error!(obligation = id.0, period, error = %e, "economy: sweep folded a miss out of order (programming error)");
                    }
                }
                // One miss ends this obligation's fire: the next period would fail for
                // the same reason and burn the grace allowance inside one sweep.
                break;
            }
            Disposition::Transient => {
                tracing::warn!(obligation = id.0, period, detail, "economy: sweep deferred a settlement");
                break;
            }
            Disposition::Human(kind) => {
                let record = page(
                    ctx,
                    kind,
                    EscalationSubject::Obligation(id),
                    format!("obligation #{id} could not settle period {period}: {detail}"),
                    obligation_advice(&o, blamed),
                    now,
                );
                if let Some(eid) = record {
                    o = suspend_for(&o, eid, clock_now);
                }
                break;
            }
        }
    }

    announce(ctx, &before, &o);
    api::put_obligation(ctx, o);
}

/// Post one period through [`api::post_with`] under the obligation's own authority,
/// with the deterministic key `obligation/{id}/{period}` — so a restart mid-sweep
/// re-derives exactly the same key and the re-post resolves to `AlreadyPosted`
/// instead of charging twice.
///
/// The third element is the account the refusal **named**, when it named one: that
/// is what turns a generic "repoint it somewhere" alert into the exact command
/// (§4.6 — the staffer should be handed the answer, not the puzzle).
fn post_period(
    ctx: &mut Ctx<'_>,
    o: &Obligation,
    period: u32,
) -> (Disposition, String, Option<AccountId>) {
    let Some(neg) = o.amount.checked_neg() else {
        return (
            Disposition::Human(EscalationKind::SettlementRefused),
            "the obligation's amount cannot be negated".to_string(),
            None,
        );
    };
    let plan = EconomyPlan {
        idempotency: obligation_key(o.id, period),
        actor: api::sweep_principal(),
        description: format!("obligation #{} period {period}", o.id),
        cause: EconomicCause::Internal { kind: SmolStr::new_static("obligation"), id: o.id.0 },
        visibility: crate::record::EconomicVisibility::default(),
        requested_lines: vec![
            RequestedLine {
                account: o.debit_account,
                delta: neg,
                memo: SmolStr::new_static("obligation"),
            },
            RequestedLine {
                account: o.credit_account,
                delta: o.amount,
                memo: SmolStr::new_static("obligation"),
            },
        ],
        expected_revisions: Vec::new(),
        asset_ops: Vec::new(),
        custody_ops: Vec::new(),
        expected_asset_revisions: Vec::new(),
        hold_ops: Vec::new(),
        expected_hold_revisions: Vec::new(),
    };
    let auth = ObligationAuth { debit: o.debit_account, credit: o.credit_account };
    match api::post_with(ctx, &plan, PostAuth::Injected(&auth)) {
        Ok(PostOutcome::Posted(t)) => {
            (Disposition::Settled(SettleOutcome::Posted(t)), String::new(), None)
        }
        Ok(PostOutcome::AlreadyPosted(t)) => {
            (Disposition::Settled(SettleOutcome::AlreadyPosted(t)), String::new(), None)
        }
        Ok(PostOutcome::NeedsApproval(request)) => (
            Disposition::Human(EscalationKind::SettlementRefused),
            format!(
                "the debit account's approval rule fired (request #{request}); the sweep cannot \
                 collect approvals"
            ),
            Some(o.debit_account),
        ),
        Err(e) => {
            // The refusal already knows which resource it is about — the same list
            // the existence-hiding renderer consults.
            let blamed = e.named_resources().into_iter().find_map(|r| match r {
                crate::plan::NamedResource::Account(a) => Some(a),
                _ => None,
            });
            (disposition(&e), e.to_string(), blamed)
        }
    }
}

/// Whether this obligation has now been forgiven for [`ForgiveCause::Overload`]
/// **twice** inside [`crate::ESCALATION_WINDOW_SECS`] (§4.6). The just-recorded event
/// counts as one, so `>= 2` means there was a prior one inside the window.
fn overload_repeats(o: &Obligation, clock_now: i64) -> bool {
    o.history
        .iter()
        .filter(|e| {
            matches!(
                e,
                ObligationEvent::Forgiven { cause: ForgiveCause::Overload, at, .. }
                    if clock_now.saturating_sub(*at) <= crate::ESCALATION_WINDOW_SECS
            )
        })
        .count()
        >= 2
}

/// Suspend an obligation pending a human decision — **this obligation alone** (§4.6).
fn suspend_for(o: &Obligation, escalation: EscalationId, at: i64) -> Obligation {
    let mut next = o.clone();
    next.status = ObligationStatus::Suspended { reason: SuspendReason::Escalated(escalation) };
    next.revision += 1;
    if next.history.len() >= crate::MAX_OBLIGATION_HISTORY {
        next.history.remove(0);
    }
    next.history.push(ObligationEvent::Escalated { id: escalation, at });
    next
}

/// The recommended commands for an obligation a human must decide about (§4.6),
/// arguments filled in. `account` names the side that refused, when it is known.
pub(crate) fn obligation_advice(o: &Obligation, account: Option<AccountId>) -> Vec<String> {
    let mut out = Vec::new();
    let side = match account {
        Some(a) if a == o.credit_account => Some("credit"),
        Some(a) if a == o.debit_account => Some("debit"),
        _ => None,
    };
    match side {
        Some(s) => out.push(format!("@obligation/repoint #{}={s} <account id>", o.id)),
        None => {
            out.push(format!("@obligation/repoint #{}=credit <account id>", o.id));
            out.push(format!("@obligation/repoint #{}=debit <account id>", o.id));
        }
    }
    out.push(format!("@obligation/cancel #{}", o.id));
    out
}

fn cause_word(c: ForgiveCause) -> &'static str {
    match c {
        ForgiveCause::Downtime => "downtime",
        ForgiveCause::TimeJump => "a time jump",
        ForgiveCause::Overload => "overload",
    }
}

// ---- notifications (§14.3, D10) -------------------------------------------

/// Tell one principal something, if it is a principal that can be told. An
/// organization or system principal has no inbox of its own; E4c's observer seam is
/// where those become staff-channel and canon output.
fn notify_principal(ctx: &mut Ctx<'_>, who: &PrincipalId, msg: &str) {
    if let PrincipalId::Object(obj) = who {
        ctx.out.sys_to_actor(*obj, "economy", msg.to_string());
    }
}

/// Notify debtor and creditor once (a self-owed obligation is told once, not twice).
fn notify_both(ctx: &mut Ctx<'_>, o: &Obligation, msg: String) {
    notify_principal(ctx, &o.debtor, &msg);
    if o.creditor != o.debtor {
        notify_principal(ctx, &o.creditor, &msg);
    }
}

/// Exception-driven output (§14.3, D10): a routine `Posted` is **silent** unless the
/// obligation asked otherwise; a transition into `Grace`, `Delinquent` or `Suspended`
/// notifies both parties once.
///
/// No message here says money moved *durably*. The sweep does not take the whole-step
/// durable gate (D-E4-6 — one gate per world step, first actor wins, so per-debtor
/// gating is not expressible), so it has no standing to make that claim; the journal
/// and `@obligation/show` are where the truth is read.
fn announce(ctx: &mut Ctx<'_>, before: &ObligationStatus, o: &Obligation) {
    if *before == o.status {
        if o.notify_on_success
            && matches!(o.history.last(), Some(ObligationEvent::Settled { .. }))
        {
            notify_both(
                ctx,
                o,
                format!("Obligation #{}: a period was submitted for settlement.", o.id),
            );
        }
        return;
    }
    let msg = match &o.status {
        ObligationStatus::Grace { missed, .. } => Some(format!(
            "Obligation #{} missed a period ({missed} missed so far) — the available funds were \
             short. It is in grace.",
            o.id
        )),
        ObligationStatus::Delinquent => Some(format!(
            "Obligation #{} is delinquent: the grace allowance is spent and the sweep will not \
             attempt it again. Staff or the creditor can reinstate it (@obligation/reinstate #{}).",
            o.id, o.id
        )),
        ObligationStatus::Suspended { reason } => Some(match reason {
            SuspendReason::Escalated(id) => format!(
                "Obligation #{} is suspended pending a staff decision (alert #{id}).",
                o.id
            ),
            SuspendReason::Staff { reason } => {
                format!("Obligation #{} is suspended: {reason}.", o.id)
            }
        }),
        ObligationStatus::Satisfied => Some(format!("Obligation #{} is satisfied.", o.id)),
        ObligationStatus::Active => {
            // Back into the pool from grace: worth one line, because the last thing
            // both parties heard was that it had missed.
            Some(format!("Obligation #{} settled and is back on schedule.", o.id))
        }
        ObligationStatus::Cancelled => None,
    };
    if let Some(msg) = msg {
        notify_both(ctx, o, msg);
    }
}

// ---- the escalation ladder ------------------------------------------------

/// Open one level-2 record (§4.6) and return its id, or `None` if the store is gone.
///
/// One record per subject: a subject that already has an **open** record is not paged
/// again, so a refusal that repeats every 30 seconds produces one alert, not a
/// thousand.
// The sweep's standing rule is **no `?` anywhere in this file** (§4.1): a reader
// auditing a tick handler should be able to grep for it and find nothing, without
// having to decide case by case whether a given `?` is an `Option` early-return or an
// error propagation that silently truncates half-done work. Clippy would rather have
// the `?` back here; the discipline is worth more than the line.
#[allow(clippy::question_mark)]
fn page(
    ctx: &mut Ctx<'_>,
    kind: EscalationKind,
    subject: EscalationSubject,
    context: String,
    recommended: Vec<String>,
    now: i64,
) -> Option<EscalationId> {
    let existing = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else { return None };
        store.escalations().find(|e| e.subject == subject && e.is_open()).map(|e| e.id)
    };
    if let Some(id) = existing {
        return Some(id);
    }
    let id = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return None };
        store.alloc_escalation()
    };
    let record = Escalation::page(id, kind, subject, context, recommended, now);
    tracing::warn!(escalation = id.0, subject = ?subject, context = %record.context, "economy: level-2 escalation opened");
    api::put_escalation(ctx, record);
    let meta = ctx.w.module_state::<EconomyStore>().map(EconomyStore::meta_doc);
    if let Some(meta) = meta {
        api::persist_meta(ctx.out, meta);
    }
    Some(id)
}

/// Level 3's companion record (§4.6): the whole-economy freeze is set, so the sweep
/// is doing nothing at all. Opened once — while one is open, later fires are silent.
fn open_freeze_record(ctx: &mut Ctx<'_>, now: i64) {
    let already = ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(|s| s.escalations().any(|e| e.subject == EscalationSubject::Economy && e.is_open()));
    if already {
        return;
    }
    let id = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        store.alloc_escalation()
    };
    // §4.6: the level-**3** action is the freeze itself (unchanged); the record it
    // now also opens is an ordinary level-2 page — a human must decide, and here is
    // the diagnosis and the command.
    let record = Escalation::page(
        id,
        EscalationKind::EconomyFrozen,
        EscalationSubject::Economy,
        "the whole-economy freeze latch is set: no posting, and the sweep is settling nothing. \
         The boot log names which projection drifted and at which sequence.",
        vec![
            "@econ/alerts".to_string(),
            "@econ/unfreeze".to_string(),
            "@account/unfreeze <id>   (per account, deliberately separate)".to_string(),
        ],
        now,
    );
    tracing::error!(escalation = id.0, "economy: the sweep is halted by the whole-economy freeze");
    api::put_escalation(ctx, record);
    let meta = ctx.w.module_state::<EconomyStore>().map(EconomyStore::meta_doc);
    if let Some(meta) = meta {
        api::persist_meta(ctx.out, meta);
    }
}

/// Re-notify about unacknowledged level-2/3 records: first after
/// [`crate::ESCALATION_RENOTIFY_SECS`] (1 h), then daily (§4.6).
///
/// The reminder is a loud log line and a stamp; the staff-channel broadcast rides
/// E4c's `EconomyObserver` glue, which is where the server (not this crate) knows how
/// to reach staff. `@econ/alerts` is the pull side and is live now.
fn renotify(ctx: &mut Ctx<'_>, now: i64) {
    let due: Vec<Escalation> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else { return };
        store
            .escalations()
            .filter(|e| e.renotify_due(now))
            .take(MAX_RENOTIFY_PER_SWEEP)
            .cloned()
            .collect()
    };
    for mut e in due {
        e.notified_at = now;
        e.notifications = e.notifications.saturating_add(1);
        e.revision = e.revision.saturating_add(1);
        tracing::warn!(
            escalation = e.id.0,
            reminders = e.notifications,
            context = %e.context,
            "economy: an escalation is still unacknowledged (@econ/ack to clear it)"
        );
        api::put_escalation(ctx, e);
    }
}

// ---- step 4: garbage collection -------------------------------------------

/// Collect what has stopped being an audit trail and started being clutter (§4.3
/// step 4): expired approval requests (E3's rule), terminal reservations past the
/// retention window, and acknowledged alerts past theirs.
///
/// **Nothing collected here can be re-minted**: the hold allocator is floored from
/// the journal, the approval and escalation allocators from every parsable key plus
/// `meta`'s high-water mark, which boot persists on every boot.
fn collect_garbage(ctx: &mut Ctx<'_>, now: i64) {
    let (approvals, holds, escalations) = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else { return };
        let snap = store.snapshot();
        let approvals: Vec<_> = snap
            .approvals()
            .filter(|r| {
                matches!(r.status, ApprovalStatus::Expired | ApprovalStatus::Cancelled)
                    && now.saturating_sub(r.expires_at) > crate::APPROVAL_GC_AFTER_SECS
            })
            .take(MAX_GC_PER_SWEEP)
            .map(|r| r.id)
            .collect();
        let holds: Vec<_> = snap
            .holds()
            .filter(|h| !h.status.is_held())
            .filter(|h| {
                // A terminal hold with no recorded terminal instant (a journal older
                // than this store's bookkeeping) is kept: never collect on a guess.
                store
                    .hold_terminal_at(h.id)
                    .is_some_and(|at| now.saturating_sub(at) > crate::HOLD_GC_AFTER_SECS)
            })
            .take(MAX_GC_PER_SWEEP)
            .map(|h| h.id)
            .collect();
        let escalations: Vec<_> = store
            .escalations()
            .filter(|e| {
                e.acked_at.is_some_and(|at| now.saturating_sub(at) > ESCALATION_GC_AFTER_SECS)
            })
            .take(MAX_GC_PER_SWEEP)
            .map(|e| e.id)
            .collect();
        (approvals, holds, escalations)
    };
    if approvals.is_empty() && holds.is_empty() && escalations.is_empty() {
        return;
    }
    {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else { return };
        for id in &approvals {
            store.snapshot.remove_approval(*id);
        }
        for id in &holds {
            store.snapshot.remove_hold(*id);
            store.hold_terminal_at.remove(id);
        }
        for id in &escalations {
            store.escalations.remove(id);
        }
    }
    for id in &approvals {
        api::delete_key(ctx.out, api::approval_key(*id));
    }
    for id in &holds {
        api::delete_key(ctx.out, api::hold_key(*id));
    }
    for id in &escalations {
        api::delete_key(ctx.out, crate::escalation::escalation_key(*id));
    }
}

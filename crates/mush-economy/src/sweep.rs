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

use std::collections::BTreeMap;

use mush_core::{Ctx, SceneId};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::api::{self, PostAuth, PostOutcome};
use crate::clock::{CadenceSource, Clocks, EconomicClock, advance_cadence, cadence_elapsed};
use crate::engine::AuthEval;
use crate::escalation::{ESCALATION_GC_AFTER_SECS, Escalation, EscalationKind, EscalationSubject};
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
/// How many **consecutive** fires may fail to expire one reservation before the
/// ladder pages a human (E4b1 verify fold, S6).
///
/// Holds had no ladder at all: an expiry the engine refuses is a level-1 log line
/// and the next fire tries again, forever, thirty seconds apart, with money reserved
/// the whole time and nobody told. Three retries is generous for anything genuinely
/// transient and short enough that a stuck reservation is somebody's problem within
/// two minutes.
const MAX_HOLD_EXPIRY_REFUSALS: u32 = 3;
/// How many **consecutive** fires may defer one obligation as transient before the
/// ladder pages a human (S7).
///
/// `Disposition::Transient` means "the next fire is the retry" — which is right, and
/// was unbounded: a journal that never comes back would have retried silently until
/// someone noticed the rent had stopped. Sixteen fires is eight minutes at the
/// shipped sweep period: long past any real blip, well short of a shift.
const MAX_CONSECUTIVE_DEFERS: u32 = 16;

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
    let evidence = Evidence {
        last_real,
        last_ic,
        now,
        ic_now,
        ratio,
        spans_boot: store.gap_spans_boot(),
    };
    // E6b (DL-09): once per boot, ask the durable canon read seam for the records
    // every live `Canon` condition cites. The RAM tail is empty after a restart, so
    // a seal committed before it would otherwise be lost to the recovery path —
    // exactly the "settlement sealed, then the server bounced" case.
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(|s| !s.canon_boot_scan_done)
    {
        if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
            store.canon_boot_scan_done = true;
        }
        crate::attest::rescan_canon_sources(ctx, crate::attest::RescanTrigger::Recovery);
    }
    advance_cadences(ctx, now, ic_now);
    expire_holds(ctx, now, ic_now);
    settle_obligations(ctx, evidence);
    forgive_suspended(ctx, evidence);
    step_contracts(ctx, now, ic_now);
    collect_garbage(ctx, now);
    // Doc 13 (W-D5): key documents for a journal older than the cut, at the
    // sweep's pace.
    crate::keys::backfill(ctx);
    // J3: the checkpoint, after the write-time drift check, at its cadence.
    crate::checkpoint::maybe_write(ctx, now);
    renotify(ctx, now);
    // E6a step 1 verify fold (R6): `Engine::reattach` fires no `Connected`, so a
    // session held across a restart never triggers the reconnect hook. Deliver every
    // standing queue whose owner is connected right now, bounded per fire.
    crate::notice::deliver_connected(ctx);
    stamp_sweep(ctx, now, ic_now);
}

/// Persist the fire's two stamps — the evidence [`classify_forgiveness`] reads to
/// tell a restart from a starved post budget.
fn stamp_sweep(ctx: &mut Ctx<'_>, now: i64, ic_now: i64) {
    let meta = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
            return;
        };
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
///
/// The per-fire cap is taken over the clocks that have **actually elapsed** (E4b1
/// verify fold, S2). Taking it over the whole list first meant a world with 64
/// clocks that never move — a plausible campaign shape, since a cadence is a season
/// or a payroll cycle — starved the 65th of every fire it ever needed: the cap
/// consumed 64 no-ops and the one clock with work to do was never reached. Filtering
/// first makes the cap what it was written to be: a bound on **work**.
fn advance_cadences(ctx: &mut Ctx<'_>, now: i64, ic_now: i64) {
    let moved: Vec<crate::clock::CadenceClock> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        store
            .snapshot()
            .cadences()
            .filter(|c| c.source != CadenceSource::Manual)
            .filter(|c| {
                let source_now = match c.source {
                    CadenceSource::RealUtc => now,
                    CadenceSource::WorldIc => ic_now,
                    CadenceSource::Manual => return false,
                };
                cadence_elapsed(c, source_now) > 0
            })
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
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        let snap = store.snapshot();
        snap.holds()
            .filter(|h| h.status.is_held())
            // Managed escrow has a coupled terminal owner. Do not consume its
            // expiry attempts or publish legacy repair advice after refusing it.
            .filter(|h| {
                !matches!(&h.purpose, crate::record::HoldPurpose::Escrow { contract }
                    if crate::contract::managed::at(snap, *contract))
            })
            .filter(|h| h.expires.is_some_and(|d| clocks.passed(&d, snap, i64::MIN)))
            // The engine refuses a hold op on a non-`Active` account (a frozen account
            // must not have its numbers moved even by an effect). Skipping here keeps
            // the sweep from posting a guaranteed refusal every 30 seconds; the hold
            // ages out as soon as the account is unfrozen.
            .filter(|h| {
                snap.account(h.account)
                    .is_some_and(|a| a.status == AccountStatus::Active)
            })
            .take(crate::MAX_HOLD_EXPIRIES_PER_SWEEP)
            .map(|h| h.id)
            .collect()
    };
    for id in due {
        let mut plan = EconomyPlan::new(
            IdempotencyKey::new(
                SmolStr::new_static("hold"),
                SmolStr::new(format!("{}/expire", id.0)),
            ),
            api::sweep_principal(),
            EconomicCause::Internal {
                kind: SmolStr::new_static("hold.expire"),
                id: id.0,
            },
            Vec::new(),
        );
        plan.hold_ops = vec![HoldOp::Expire { hold: id }];
        // Exhaustive, like every outcome in this file.
        let refusal = match api::post_with(ctx, &plan, PostAuth::World) {
            Ok(PostOutcome::Posted(_) | PostOutcome::AlreadyPosted(_)) => None,
            Ok(PostOutcome::NeedsApproval(request)) => {
                // Unreachable: an effect-only expiry has no net debit, so no
                // `ApprovalRule` can fire on it. Loud rather than silent.
                tracing::error!(
                    hold = id.0,
                    request = request.0,
                    "economy: a hold expiry asked for approval"
                );
                Some(format!(
                    "the expiry asked for approval (request #{request})"
                ))
            }
            Err(e) => {
                // A hold whose deadline passed but whose expiry is refused is not a
                // money problem *yet*: the reservation simply stands until the
                // refusal is fixed. Level 1 — inform — for the first few fires.
                tracing::warn!(hold = id.0, error = %e, "economy: sweep could not expire a hold");
                Some(e.to_string())
            }
        };
        note_expiry_outcome(ctx, id, refusal, now);
    }
}

/// Fold one expiry attempt into the hold's ladder run (S6).
///
/// A success clears the run. A refusal extends it, and past
/// [`MAX_HOLD_EXPIRY_REFUSALS`] consecutive fires the reservation gets its own
/// level-2 record: somebody's money has been unavailable for minutes with a deadline
/// that has already passed, which is exactly the "a human must decide" rung.
fn note_expiry_outcome(ctx: &mut Ctx<'_>, id: HoldId, refusal: Option<String>, now: i64) {
    let Some(detail) = refusal else {
        if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
            store.hold_expiry_failures.remove(&id);
        }
        return;
    };
    let (run, exhausted) = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
            return;
        };
        bump_run(
            &mut store.hold_expiry_failures,
            id,
            MAX_HOLD_EXPIRY_REFUSALS,
        )
    };
    if !exhausted {
        return;
    }
    let account = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().hold(id).map(|h| h.account));
    let mut recommended = vec![format!("@hold/show #{id}"), format!("@hold/release #{id}")];
    if let Some(a) = account {
        recommended.push(format!(
            "@account/unfreeze {a}   (if that is why it will not age out)"
        ));
    }
    page(
        ctx,
        EscalationKind::SettlementRefused,
        EscalationSubject::Hold(id),
        account,
        format!(
            "hold #{id}'s deadline has passed but the sweep has failed to age it out {run} times \
             running: {detail}. The money stays unavailable until this is resolved"
        ),
        recommended,
        now,
    );
}

/// Extend a consecutive-failure run and say whether it has crossed `cap`.
///
/// The whole of the S6/S7 bookkeeping, in one place and pure enough to test: a run
/// counts *consecutive* failures, saturates rather than wrapping, and reports
/// "crossed" exactly once per crossing — the caller's `page` dedupes the rest.
fn bump_run<K: Ord + Copy>(runs: &mut BTreeMap<K, u32>, key: K, cap: u32) -> (u32, bool) {
    let run = runs.entry(key).or_insert(0);
    *run = run.saturating_add(1);
    (*run, *run > cap)
}

// ---- step 2: obligations --------------------------------------------------

/// One obligation's narrow settlement authority.
///
/// The sweep is not staff and holds nobody's wallet rights: this authority permits
/// **exactly** the two lines the obligation itself declares, and nothing else. Every
/// other [`AuthEval`] method keeps its default `false`, so a settlement plan can move
/// no title, no custody, and no reservation.
///
/// A `System` debit is refused unless the obligation was **minted by the ruleset's
/// own faucet authority** and its per-period amount is inside the live
/// [`crate::EconomyConfig`] cap (M1). Debiting a registered faucet creates money from
/// nothing, so `@obligation/create` refuses the shape outright and there is no
/// command that does not; the only way in is
/// [`crate::create_system_obligation`], and even then this seam re-checks the cap at
/// **every** settlement — a doc edited above the cap after creation is refused here,
/// not paid.
///
/// **The org-liveness gate is in this seam** (E4b1 verify fold, S1). E4b1 shipped
/// `authorize` as a pure identity check, so a standing obligation kept settling into
/// and out of a `Suspended` compartment that every *manual* payment was refused for —
/// the sweep was the one payer in the world that org status did not bind, and a
/// wound-down division kept collecting rent forever. The two flags are computed
/// against the live world immediately before the post ([`api::scope_coherent`] for
/// the debit side, [`api::credit_allowed`] for the credit side — the very predicates
/// `WorldAuth` applies) and carried in, because `WorldAuth` borrows the world and a
/// caller holding `&mut Ctx` cannot build one to hand over (§4.1's `PostAuth` note).
/// [`post_period`] checks them first so the refusal can name the blamed account; this
/// is the lock that holds if a future caller forgets to.
struct ObligationAuth {
    debit: AccountId,
    credit: AccountId,
    debit_live: bool,
    credit_live: bool,
    /// The obligation carries a `minted_by` faucet AND its amount is within the live
    /// mint cap — the one and only route to a `System` debit (M1).
    mint_allowed: bool,
}

impl AuthEval for ObligationAuth {
    /// E6a step 1 verify fold (M1): a period post is **not discretionary spend**.
    ///
    /// The debit was consented to when the obligation was created, and the sweep can
    /// collect no approval — so an account whose rolling [`crate::ApprovalWindow`] is
    /// already full would have had its rent refused, the record suspended and a
    /// level-2 page opened, with the creditor unpaid and nothing anybody could do but
    /// widen the window. The **threshold** still fires (an obligation above the
    /// per-transaction limit escalates exactly as before), and the post still counts
    /// toward the window for every later discretionary spend on the same account.
    fn window_exempt(&self) -> bool {
        true
    }

    fn authorize(&self, _actor: &PrincipalId, account: &Account, delta: Amount) -> bool {
        if delta.is_negative() {
            if account.id != self.debit || !self.debit_live {
                return false;
            }
            if matches!(account.owner, PrincipalId::System(_)) {
                return self.mint_allowed;
            }
            return true;
        }
        account.id == self.credit && self.credit_live
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
        EconomyError::JournalUnavailable
        | EconomyError::EconomyFrozen
        | EconomyError::RetryShortly => Disposition::Transient,
        // A settlement refusal only a human can fix (§4.3): a closed/missing/frozen
        // account, a retired instrument, an inactive org compartment, an approval rule
        // the sweep cannot satisfy, or a deterministic-key conflict (doc 09 C8's
        // integrity incident, scoped to this obligation alone).
        EconomyError::NoSuchAccount(_)
        | EconomyError::InactiveAccount(_)
        | EconomyError::NoSuchInstrument(_)
        | EconomyError::InactiveInstrument(_)
        | EconomyError::InstrumentExists(_)
        | EconomyError::ObligationTerminal(_)
        | EconomyError::Unauthorized
        | EconomyError::ScopeOwnerMismatch
        | EconomyError::ApprovalRequired
        | EconomyError::ApprovalExpired
        | EconomyError::Overflow
        // M1: a standing mint above its configured cap is exactly a human decision —
        // raise the cap deliberately, or lower the obligation.
        | EconomyError::MintCapExceeded(_) => Disposition::Human(EscalationKind::SettlementRefused),
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
        | EconomyError::TransactionNotInMemory(_)
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
        | EconomyError::AlreadyClaimed(_)
        | EconomyError::NoSuchVault(_)
        | EconomyError::VaultNotActive(_)
        | EconomyError::NotInVault(_)
        | EconomyError::NoCustodyStanding(_)
        | EconomyError::BadDestination(_)
        | EconomyError::BackingNotDepositable(_)
        | EconomyError::BackingInvalid(_)
        | EconomyError::DuplicateAssetOp(_)
        | EconomyError::DuplicateCustodyOp(_)
        | EconomyError::ManagedContract
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
fn settle_obligations(ctx: &mut Ctx<'_>, ev: Evidence) {
    let due: Vec<ObligationId> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
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
        settle_one(ctx, id, ev, &mut posts);
    }
}

/// **A period that falls due while an obligation is suspended is forgiven as it
/// falls due** (look-back fold, D-LBK-1), recorded `Forgiven { Suspension }` in the
/// obligation's own history and never charged.
///
/// Before the fold a suspended obligation simply left the due pool, and the gap it
/// accumulated was the resumer's problem: `@obligation/resume` re-anchored it, but
/// `@econ/ack` did not, so the sweep after an acknowledgement charged
/// `maximum_catch_up` periods of a suspension it had promised were "forgiven by the
/// next sweep". Forgiving here, every fire, makes that sentence true for every holder
/// — the ladder's, staff's, and a layered module's — and leaves nothing for a
/// release to charge. Bounded like the settlement pass: at most
/// [`crate::MAX_OBLIGATIONS_PER_SWEEP`] records written per fire.
fn forgive_suspended(ctx: &mut Ctx<'_>, ev: Evidence) {
    let clocks = Clocks::new(ev.now, ev.ic_now);
    let gaps: Vec<(ObligationId, u32, i64)> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        let snap = store.snapshot();
        snap.obligations()
            .filter(|o| matches!(o.status, ObligationStatus::Suspended { .. }))
            .filter_map(|o| {
                let clock_now = clocks.now_on(o.schedule.clock, snap, o.schedule.manual_now)?;
                let due = crate::clock::due_periods(&o.schedule, clock_now);
                (due > 0).then_some((o.id, due, clock_now))
            })
            .take(crate::MAX_OBLIGATIONS_PER_SWEEP)
            .collect()
    };
    for (id, due, clock_now) in gaps {
        let Some(o) = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().obligation(id).cloned())
        else {
            continue;
        };
        // Wholesale, consuming no period index: the schedule re-anchors past `now`
        // and no deterministic key is reused (finding 13's shape).
        let next = forgive_all(&o, due, ForgiveCause::Suspension, clock_now);
        notify_both(
            ctx,
            &next,
            format!(
                "Obligation #{id}: {due} period(s) fell due while it was suspended; forgiven, \
                 not charged."
            ),
        );
        api::put_obligation(ctx, next);
    }
}

/// Everything [`classify_forgiveness`] needs, carried as one value: the two persisted
/// sweep stamps, the two live instants, the ruleset's IC ratio, and (E4b1 verify
/// fold, S3) whether a **boot** happened inside the gap.
#[derive(Copy, Clone)]
struct Evidence {
    last_real: i64,
    last_ic: i64,
    now: i64,
    ic_now: i64,
    ratio: i64,
    spans_boot: bool,
}

/// One obligation's whole sweep step.
fn settle_one(ctx: &mut Ctx<'_>, id: ObligationId, ev: Evidence, posts: &mut usize) {
    let (now, ic_now) = (ev.now, ev.ic_now);
    let clocks = Clocks::new(now, ic_now);
    let found = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        let snap = store.snapshot();
        snap.obligation(id).map(|o| {
            (
                o.clone(),
                clocks.now_on(o.schedule.clock, snap, o.schedule.manual_now),
            )
        })
    };
    let Some((mut o, clock_now)) = found else {
        return;
    };
    // A schedule on a cadence whose record is gone resolves to `None`, and every
    // consumer reads that as **not yet** — which for a clock that will never be
    // readable again means "never", silently, forever (E4b1 verify fold, B6). Boot
    // asks the same question; this is the arm for a cadence that disappears while the
    // world is up.
    let Some(clock_now) = clock_now else {
        page_unresolvable_clock(ctx, o, now);
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
        let cause = classify_forgiveness(
            ev.last_real,
            ev.last_ic,
            ev.now,
            ev.ic_now,
            ev.ratio,
            ev.spans_boot,
        );
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
            None,
            format!(
                "obligation #{id} has been forgiven for overload twice inside the escalation \
                 window: it keeps missing its post budget, which usually means its cadence is \
                 far shorter than the sweep period or the pool is starved"
            ),
            obligation_advice(&o, None, Blocker::Unknown),
            now,
        );
        o = suspend_paged(&o, record, clock_now);
        notify_both(ctx, &o, suspension_notice(id, record));
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
        if !matches!(disposition, Disposition::Transient) {
            // Any other outcome ends the transient run: the journal answered (S7).
            if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
                store.obligation_defers.remove(&id);
            }
        }
        match disposition {
            Disposition::Settled(outcome) => {
                match advance_obligation(&o, period, outcome, clock_now) {
                    Ok(next) => o = next,
                    Err(e) => {
                        // `OutOfOrder` (and a re-delivered duplicate, which `advance`
                        // absorbs) can only arise from a bug in THIS file's ordering, not
                        // from anything the world did. Loud, and never a panic: a panic
                        // here would take the world task down over a bookkeeping slip.
                        tracing::error!(obligation = id.0, period, error = %e, "economy: sweep folded a settlement outcome out of order (programming error)");
                        break;
                    }
                }
            }
            Disposition::Short => {
                match advance_obligation(
                    &o,
                    period,
                    SettleOutcome::InsufficientAvailable,
                    clock_now,
                ) {
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
                tracing::warn!(
                    obligation = id.0,
                    period,
                    detail,
                    "economy: sweep deferred a settlement"
                );
                // S7: "retry next fire" is only an answer while the retries are
                // bounded. Past the cap this stops being a blip and becomes a
                // decision, and the obligation leaves the due pool rather than
                // retrying silently for the rest of the world's life.
                let (run, exhausted) = {
                    let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
                        break;
                    };
                    bump_run(&mut store.obligation_defers, id, MAX_CONSECUTIVE_DEFERS)
                };
                if exhausted {
                    let record = page(
                        ctx,
                        EscalationKind::SettlementRefused,
                        EscalationSubject::Obligation(id),
                        blamed,
                        format!(
                            "obligation #{id} has been deferred {run} fires running on a refusal \
                             that was classified transient: {detail}. It is no longer plausibly \
                             transient"
                        ),
                        obligation_advice(&o, blamed, Blocker::Unknown),
                        now,
                    );
                    o = suspend_paged(&o, record, clock_now);
                }
                break;
            }
            Disposition::Human(kind) => {
                let blocker = blocker_for(ctx, blamed);
                let record = page(
                    ctx,
                    kind,
                    EscalationSubject::Obligation(id),
                    blamed,
                    format!("obligation #{id} could not settle period {period}: {detail}"),
                    obligation_advice(&o, blamed, blocker),
                    now,
                );
                o = suspend_paged(&o, record, clock_now);
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
    // ---- S1 + M1: the two gates that are the SEAM's, asked before the post ----
    //
    // `ObligationAuth` enforces both (that is the lock); asking here as well is what
    // lets the refusal name the account it is about, which is the difference between
    // an alert a staffer can act on and one they have to investigate.
    let gate = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return (
                Disposition::Transient,
                "the economy store is not loaded".to_string(),
                None,
            );
        };
        let snap = store.snapshot();
        let debit = snap.account(o.debit_account).cloned();
        let credit = snap.account(o.credit_account).cloned();
        let cap = debit
            .as_ref()
            .and_then(|a| store.config().mint_cap(snap, &a.instrument));
        (debit, credit, cap)
    };
    let (debit, credit, mint_cap) = gate;
    let debit_live = debit
        .as_ref()
        .is_none_or(|a| api::scope_coherent(ctx.w, a.scope, &a.owner));
    let credit_live = credit
        .as_ref()
        .is_none_or(|a| api::credit_allowed(ctx.w, a));
    if !debit_live {
        return (
            Disposition::Human(EscalationKind::SettlementRefused),
            "the debit account's organization or compartment is not active, so the sweep may not \
             take money out of it"
                .to_string(),
            Some(o.debit_account),
        );
    }
    if !credit_live {
        return (
            Disposition::Human(EscalationKind::SettlementRefused),
            "the credit account's organization may no longer receive value".to_string(),
            Some(o.credit_account),
        );
    }
    // M1: a faucet debit is allowed only for a ruleset-minted obligation, and only
    // while its per-period amount is inside the LIVE cap — re-read every settlement,
    // so a doc edited above the cap after creation is refused rather than paid.
    let faucet_debit = debit
        .as_ref()
        .is_some_and(|a| matches!(a.owner, PrincipalId::System(_)));
    let mint_allowed = o.minted_by.is_some() && mint_cap.is_some_and(|cap| o.amount <= cap);
    if faucet_debit && !mint_allowed {
        let detail = if o.minted_by.is_none() {
            "its debit side is a system source/sink but nothing minted it: a standing mint needs \
             the ruleset's faucet authority (create_system_obligation), never a command"
                .to_string()
        } else {
            // The cap in the instrument's own scale and symbol (E6a step 1, E2-C16),
            // read through the store's snapshot; an unset cap says so.
            let rendered = mint_cap.map_or_else(
                || "unset".to_string(),
                |cap| {
                    // The alert is the staff channel's, and the record's own debit
                    // account is the instrument it is denominated in; M8's fallback
                    // is what keeps a raw minor value out of the page when the
                    // account itself has gone.
                    ctx.w.module_state::<EconomyStore>().map_or_else(
                        || cap.to_decimal_string(0),
                        |s| {
                            let instrument = s
                                .snapshot()
                                .account(o.credit_account)
                                .map(|a| a.instrument.clone());
                            crate::render::render_amount_on(
                                s.snapshot(),
                                o.debit_account,
                                cap,
                                true,
                                instrument.as_ref(),
                            )
                        },
                    )
                },
            );
            format!(
                "its per-period amount is above the configured mint cap ({rendered}); \
                 raise [economy] mint_obligation_cap_major or lower the obligation"
            )
        };
        return (
            Disposition::Human(EscalationKind::SettlementRefused),
            detail,
            Some(o.debit_account),
        );
    }
    let plan = EconomyPlan {
        idempotency: obligation_key(o.id, period),
        actor: api::sweep_principal(),
        description: format!("obligation #{} period {period}", o.id),
        cause: EconomicCause::Internal {
            kind: SmolStr::new_static("obligation"),
            id: o.id.0,
        },
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
    let auth = ObligationAuth {
        debit: o.debit_account,
        credit: o.credit_account,
        debit_live,
        credit_live,
        mint_allowed,
    };
    match api::post_with(ctx, &plan, PostAuth::Injected(&auth)) {
        Ok(PostOutcome::Posted(t)) => (
            Disposition::Settled(SettleOutcome::Posted(t)),
            String::new(),
            None,
        ),
        Ok(PostOutcome::AlreadyPosted(t)) => (
            Disposition::Settled(SettleOutcome::AlreadyPosted(t)),
            String::new(),
            None,
        ),
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
///
/// `record` is `None` only when the escalation allocator itself is exhausted (E4b1
/// verify fold, B2). The subject still stops: an alert that could not be minted is a
/// reason to page harder, never a reason to keep charging an account nobody can fix.
fn suspend_paged(o: &Obligation, record: Option<EscalationId>, at: i64) -> Obligation {
    // The ladder holds under its own key (look-back fold, D-LBK-1): `@econ/ack`
    // releases exactly this key, and a layered module's hold on the same obligation
    // neither lifts it nor is lifted by it.
    let reason = match record {
        Some(id) => SuspendReason::Escalated(id),
        None => SuspendReason::Staff {
            reason: SmolStr::new_static("alert-allocator-exhausted"),
        },
    };
    let mut next = crate::obligation::hold(o, reason, at);
    if next.history.len() >= crate::MAX_OBLIGATION_HISTORY {
        next.history.remove(0);
    }
    if let Some(id) = record {
        next.history.push(ObligationEvent::Escalated { id, at });
    }
    next
}

/// The line both parties are told when their obligation stops.
fn suspension_notice(id: ObligationId, record: Option<EscalationId>) -> String {
    match record {
        Some(eid) => format!("Obligation #{id} is suspended pending staff (alert #{eid})."),
        None => format!("Obligation #{id} is suspended pending staff."),
    }
}

/// What is actually blocking a settlement, when the refusal named an account — the
/// difference between advice that fixes the problem and advice that does not.
#[derive(Copy, Clone, PartialEq, Eq)]
pub(crate) enum Blocker {
    /// Nothing more specific is known: the account is closed, gone, or refused for a
    /// reason the sweep cannot name.
    Unknown,
    /// The blamed account carries a **per-account recovery freeze** and its owner is
    /// still here (E4b1 verify fold, B4b). Repointing an obligation away from a
    /// recoverable frozen wallet is precisely the wrong move — it abandons the money
    /// sitting in it and re-aims a live debt at some other account — and a recovery
    /// boot that froze fifty accounts would otherwise have produced fifty alerts all
    /// recommending exactly that.
    ///
    /// A frozen account whose **owner is gone** is a different problem with a
    /// different answer, and stays `Unknown`: unfreezing it would settle money into a
    /// destroyed principal's wallet, so repointing really is the fix there.
    FrozenAccount,
}

/// Which [`Blocker`] the blamed account represents right now.
fn blocker_for(ctx: &Ctx<'_>, blamed: Option<AccountId>) -> Blocker {
    let Some(id) = blamed else {
        return Blocker::Unknown;
    };
    let recoverable = ctx.w.module_state::<EconomyStore>().is_some_and(|s| {
        s.snapshot().account(id).is_some_and(|a| {
            a.status == AccountStatus::Frozen && crate::boot::principal_live(ctx.w, &a.owner)
        })
    });
    if recoverable {
        Blocker::FrozenAccount
    } else {
        Blocker::Unknown
    }
}

/// The recommended commands for an obligation a human must decide about (§4.6),
/// arguments filled in. `account` names the side that refused, when it is known.
pub(crate) fn obligation_advice(
    o: &Obligation,
    account: Option<AccountId>,
    blocker: Blocker,
) -> Vec<String> {
    let mut out = Vec::new();
    if let (Blocker::FrozenAccount, Some(a)) = (blocker, account) {
        // B4b: the account exists, holds its money, and is one command from working.
        out.push(format!("@account/unfreeze {a}"));
        out.push("@econ/ack #<this alert>   (resumes it once the account is back)".to_string());
        out.push(format!("@obligation/cancel #{}", o.id));
        return out;
    }
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

/// B6's live arm: the schedule's clock cannot be resolved, so no period can ever fall
/// due. Suspend the subject and page, exactly as boot does for the same shape.
fn page_unresolvable_clock(ctx: &mut Ctx<'_>, o: Obligation, now: i64) {
    if !o.status.in_due_pool() {
        return;
    }
    let id = o.id;
    let clock = match o.schedule.clock {
        EconomicClock::Cadence(c) => format!("cadence clock #{c}, which has no record"),
        other => format!("{other:?}, which cannot be resolved"),
    };
    let record = page(
        ctx,
        EscalationKind::SettlementRefused,
        EscalationSubject::Obligation(id),
        None,
        format!(
            "obligation #{id}'s schedule runs on {clock}: no period can fall due on it, so it \
             would have sat in the due pool charging nothing and telling nobody"
        ),
        vec![
            format!("@obligation/show #{id}"),
            format!("@obligation/cancel #{id}"),
            "@econ/ack #<this alert>   (resumes it once the clock exists again)".to_string(),
        ],
        now,
    );
    let next = suspend_paged(&o, record, now);
    notify_both(ctx, &next, suspension_notice(id, record));
    api::put_obligation(ctx, next);
}

fn cause_word(c: ForgiveCause) -> &'static str {
    match c {
        ForgiveCause::Downtime => "downtime",
        ForgiveCause::TimeJump => "a time jump",
        ForgiveCause::Overload => "overload",
        ForgiveCause::Suspension => "a suspension",
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
        if o.notify_on_success && matches!(o.history.last(), Some(ObligationEvent::Settled { .. }))
        {
            notify_both(
                ctx,
                o,
                format!(
                    "Obligation #{}: a period was submitted for settlement.",
                    o.id
                ),
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
            SuspendReason::Held { key } => {
                format!("Obligation #{} is suspended: held by {key}.", o.id)
            }
        }),
        ObligationStatus::Satisfied => Some(format!("Obligation #{} is satisfied.", o.id)),
        ObligationStatus::Active => {
            // Back into the pool from grace: worth one line, because the last thing
            // both parties heard was that it had missed.
            Some(format!(
                "Obligation #{} settled and is back on schedule.",
                o.id
            ))
        }
        ObligationStatus::Cancelled => None,
    };
    if let Some(msg) = msg {
        notify_both(ctx, o, msg);
    }
}

// ---- step 3: contracts (E4b2, §4.3) ---------------------------------------

/// The most contracts one fire touches (§9's list discipline, applied to work).
const MAX_CONTRACTS_PER_SWEEP: usize = 64;

/// What this fire must do about one contract.
///
/// Decided in a **read-only** pass and then applied by one exhaustive `match`, for
/// the same reason every other step in this file is shaped that way: the decision is
/// made against one immutable view of the world, and the mutating pass has no
/// lookups of its own that could see a half-changed state.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ContractStep {
    /// A terminal transition whose plan has not posted yet (E4b2 verify fold,
    /// M2+M3): retry it. This step comes **before** every other one — a contract on
    /// its way out must not settle, expire again, or be paged as a stalled dispute.
    Retry,
    /// Every condition holds: promote it if it is still `Active`, and settle.
    Settle,
    /// Its deadline has passed unsettled: apply `on_expiry` (refund / forfeit /
    /// dispute).
    Expire,
    /// Disputed for longer than [`crate::DISPUTE_TTL_SECS`] with no unanimous party
    /// agreement: page a human, with the resolving command filled in.
    Page,
    /// A `Draft` nobody finished within [`crate::DRAFT_TTL_SECS`]: collect it. A
    /// draft has no acceptances and therefore no escrow, so there is nothing to give
    /// back.
    Collect,
}

/// The contract step (§4.3 step 3, D-E4-3).
///
/// Every deadline is read **on its own clock**, and a clock this sweep cannot
/// resolve (a `Manual` contract deadline, a cadence with no record) reads as *not
/// passed* — the same fail-closed rule hold expiry uses. A contract must never lapse
/// because nobody could tell what time it was.
///
/// **The per-fire cap bounds work, not the scan** (E4b2 verify fold, B4). Two rules
/// keep it from starving a contract that genuinely needs the fire:
///
/// 1. **Nothing-due records are filtered out before the cap.** A `Disputed` contract
///    past `DISPUTE_TTL_SECS` used to yield `Page` on every fire forever; `page`
///    deduped it to a no-op, but it had already spent one of the sixty-four slots. So
///    a `Page` counts only when its record is actually due to say something — there
///    is no record yet, or it is due to re-notify, or it is an acknowledged one whose
///    cause has come back.
/// 2. **The scan rotates.** Even with only real work in the pool, `contracts()` is in
///    id order, so a sixty-fifth contract would never be reached. The cursor makes
///    the pass a rotation, so every record is reached within one sweep of the pool.
fn step_contracts(ctx: &mut Ctx<'_>, now: i64, ic_now: i64) {
    // Doc 13 W2 (W-D7): what boot left for the store to decide.
    crate::contract_live::heal_lost_transitions(ctx);
    stamp_observed_conditions(ctx, now, ic_now);
    republish_settlements(ctx, now);
    let clocks = Clocks::new(now, ic_now);
    let due: Vec<(crate::ids::ContractId, ContractStep)> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        let providers = store.providers();
        let custody = crate::custody::WorldCustody {
            w: ctx.w,
            providers,
        };
        let facts = crate::facts::scoped::ContextualWorldFacts {
            ctx,
            providers: store.facts(),
        };
        let world = api::WorldAuth { w: ctx.w };
        let gate = store.key_gate();
        let env = crate::engine::ValidateEnv {
            keys: &gate,
            auth: &world,
            custody: &custody,
            facts: &facts,
            now,
            ic_now,
            next_hold_id: store.peek_next_hold(),
            spent: std::collections::BTreeMap::new(),
        };
        let snap = store.snapshot();
        let lapsed = |c: &crate::contract::EconomicContract| {
            c.consent
                .deadline
                .is_some_and(|d| clocks.passed(&d, snap, i64::MIN))
        };
        // Rule 1: a dispute past its TTL is work only when the ladder has something
        // to say about it this fire.
        let page_due = |id: crate::ids::ContractId| match store
            .escalations()
            .find(|e| e.subject == EscalationSubject::Contract(id))
        {
            None => true,
            Some(e) => e.renotify_due(now) || e.reopen_due(now),
        };
        snap.contracts()
            .filter(|contract| contract.consent.managed.is_none())
            .filter_map(|c| {
                // A pending terminal transition outranks everything: its plan may
                // already be durable, and nothing else may run over it.
                // (A terminal contract carrying one is boot's orphan-escrow
                // recovery — it still has money to hand back.)
                if c.pending.is_some() {
                    return Some((c.id, ContractStep::Retry));
                }
                let step = match c.state {
                    // E6a step 1 (M10): a `Draft` can hold live escrow between an A′
                    // amendment and its re-offer. Past its own deadline that escrow is
                    // money nobody is going to fund an agreement with — unwind it
                    // exactly as an `Offered` contract lapses. A draft holding nothing
                    // is collected on the TTL as before; one holding escrow is never
                    // collected (`collect_draft` skips it), only expired.
                    crate::contract::ContractState::Draft => {
                        let escrowed =
                            !crate::contract::qualifying_escrow(c, snap).holds.is_empty();
                        if escrowed {
                            // E6a step 1 verify fold (M2): a post-A′ draft holding
                            // escrow with **no deadline** had no route out at all —
                            // `lapsed` needs a deadline, `collect_draft` skips an
                            // escrowed draft, and `page_stale_contracts` looked only
                            // at live states. The payer's money sat reserved forever
                            // with nothing scheduled to touch it. The draft TTL is the
                            // backstop: the unwind refunds the payer, who may re-offer.
                            let untouched_past_ttl = c.consent.deadline.is_none()
                                && now.saturating_sub(c.created_at) > crate::DRAFT_TTL_SECS;
                            (lapsed(c) || untouched_past_ttl).then_some(ContractStep::Expire)
                        } else {
                            (now.saturating_sub(c.created_at) > crate::DRAFT_TTL_SECS)
                                .then_some(ContractStep::Collect)
                        }
                    }
                    crate::contract::ContractState::Offered => {
                        lapsed(c).then_some(ContractStep::Expire)
                    }
                    // Readiness first: a contract whose conditions hold should settle
                    // rather than lapse, even on the fire its deadline passes. That is
                    // also where a `Before`/`After` condition, or a re-verification
                    // that was deferred last fire, finally comes true.
                    crate::contract::ContractState::Active
                    | crate::contract::ContractState::ReadyToSettle => {
                        match crate::contract::readiness(c, snap, &env) {
                            // `Undecidable` goes down the same arm on purpose: the
                            // settlement path is what turns "this cannot settle at
                            // all" into a `Disputed` contract and a level-2 page.
                            crate::contract::Readiness::Ready(_)
                            | crate::contract::Readiness::Undecidable(_) => {
                                Some(ContractStep::Settle)
                            }
                            crate::contract::Readiness::NotYet => {
                                lapsed(c).then_some(ContractStep::Expire)
                            }
                        }
                    }
                    crate::contract::ContractState::Disputed => (c
                        .disputed_at
                        .is_some_and(|at| now.saturating_sub(at) > crate::DISPUTE_TTL_SECS)
                        && page_due(c.id))
                    .then_some(ContractStep::Page),
                    crate::contract::ContractState::Settled { .. }
                    | crate::contract::ContractState::Cancelled
                    | crate::contract::ContractState::Expired => None,
                };
                step.map(|s| (c.id, s))
            })
            .collect()
    };
    // Rule 2: rotate the (id-ordered) due list so the scan resumes where it stopped.
    let cursor = ctx
        .w
        .module_state::<EconomyStore>()
        .map_or(0, |s| s.sweep_contract_cursor);
    let mut due = due;
    let split = due
        .iter()
        .position(|(id, _)| id.0 >= cursor)
        .unwrap_or(due.len());
    due.rotate_left(split);
    let work: Vec<(crate::ids::ContractId, ContractStep)> =
        due.into_iter().take(MAX_CONTRACTS_PER_SWEEP).collect();
    let next_cursor = work.last().map_or(0, |(id, _)| id.0.saturating_add(1));
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.sweep_contract_cursor = next_cursor;
    }
    for (id, step) in work {
        match step {
            // The plan-first rule (M2+M3): one more attempt at the transition. Every
            // outcome is handled inside — a refusal counts an attempt, pages, and
            // leaves the contract live with its escrow intact; the last one disputes.
            ContractStep::Retry => {
                let _ = crate::contract_live::finish_terminal(ctx, id);
            }
            ContractStep::Settle => {
                // Every outcome of a settlement attempt is already handled inside:
                // a refusal leaves the contract `ReadyToSettle` with a loud log, and
                // `MAX_SETTLE_ATTEMPTS` of them make it `Disputed` and page.
                let _ = crate::contract_live::promote_and_settle(ctx, id, true);
            }
            ContractStep::Expire => crate::contract_live::expire(ctx, id),
            ContractStep::Page => {
                page(
                    ctx,
                    EscalationKind::SettlementRefused,
                    EscalationSubject::Contract(id),
                    None,
                    format!(
                        "contract #{id} has been disputed for more than {} days with no unanimous \
                         party agreement; its escrow is frozen the whole time",
                        crate::DISPUTE_TTL_SECS / 86_400
                    ),
                    vec![
                        format!("@contract/show #{id}"),
                        format!("@contract/resolve #{id}=settle"),
                        format!("@contract/resolve #{id}=refund"),
                    ],
                    now,
                );
            }
            ContractStep::Collect => collect_draft(ctx, id),
        }
    }
    page_stale_contracts(ctx, now, ic_now);
}

/// Stamp every live contract's deterministic conditions that hold but carry no
/// observation yet (E4c verify fold, E5).
///
/// It runs **before** the due-list is computed, so an `After` anchored on a `Fact`
/// sees its anchor on the same fire the fact becomes true. The stamp is not evidence:
/// `condition_holds` re-evaluates the condition against the live world at settlement
/// exactly as before, so a fact that stops holding still stops settling. Bounded by
/// the same [`MAX_CONTRACTS_PER_SWEEP`] budget as everything else here.
fn stamp_observed_conditions(ctx: &mut Ctx<'_>, now: i64, ic_now: i64) {
    let stamped: Vec<crate::contract::EconomicContract> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        let providers = store.providers();
        let custody = crate::custody::WorldCustody {
            w: ctx.w,
            providers,
        };
        let facts = crate::facts::scoped::ContextualWorldFacts {
            ctx,
            providers: store.facts(),
        };
        let world = api::WorldAuth { w: ctx.w };
        let gate = store.key_gate();
        let env = crate::engine::ValidateEnv {
            keys: &gate,
            auth: &world,
            custody: &custody,
            facts: &facts,
            now,
            ic_now,
            next_hold_id: store.peek_next_hold(),
            spent: std::collections::BTreeMap::new(),
        };
        let snap = store.snapshot();
        snap.contracts()
            .filter(|contract| contract.consent.managed.is_none())
            .filter(|c| {
                matches!(
                    c.state,
                    crate::contract::ContractState::Active
                        | crate::contract::ContractState::ReadyToSettle
                        | crate::contract::ContractState::Disputed
                )
            })
            .filter_map(|c| {
                let mut next = c.clone();
                crate::contract::stamp_observed(&mut next, snap, &env).then(|| {
                    next.revision = next.revision.saturating_add(1);
                    next
                })
            })
            .take(MAX_CONTRACTS_PER_SWEEP)
            .collect()
    };
    for c in stamped {
        if let Err(error) = api::put_contract(ctx, c) {
            tracing::error!(%error, "economy: legacy contract document refused before publication");
            return;
        }
    }
}

/// How many settlements one sweep fire re-announces to the observers (E4c verify
/// fold, V2+V3+V5).
pub const MAX_REPUBLISH_PER_SWEEP: usize = 8;

/// Re-announce settled `Public` contracts that never reached the chronicle
/// (E4c verify fold, V2 + V3 + V5).
///
/// Publication was best-effort in three separate ways, and they share one cure. The
/// bridge's `mush_canon::commit` could return `None` — an exhausted id allocator, a
/// ledger append the store refused — and the line was simply lost, with a `let _ =`
/// where the error should have been. Boot's `finished_transition` moves a contract to
/// `Settled` by writing the document directly, so no event is ever emitted and a
/// settlement recovered at boot was *never* chronicled. And the replay guard read
/// canon's RAM tail, which rolls.
///
/// The cure is [`crate::contract::EconomicContract::canon_published`]: a **durable** marker the bridge
/// writes only after a commit succeeded. Anything settled, public, older than a sweep
/// period and still unmarked is announced again — the bridge de-dupes on the marker,
/// so a line that did land is never doubled, and one that did not is retried until it
/// does.
fn republish_settlements(ctx: &mut Ctx<'_>, now: i64) {
    let due: Vec<(crate::ids::ContractId, crate::ids::TransactionId)> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        let snap = store.snapshot();
        snap.contracts()
            .filter(|contract| contract.consent.managed.is_none())
            .filter(|c| c.canon_published.is_none() && c.consent.visibility.is_public())
            .filter_map(|c| match c.state {
                crate::contract::ContractState::Settled { transaction } => {
                    Some((c.id, transaction))
                }
                _ => None,
            })
            // **At least one sweep old.** A settlement that just happened has already
            // been announced once, in its own step; re-announcing it on the very next
            // fire would race the bridge's own marker write rather than recover from
            // its failure.
            .filter(|(_, tx)| {
                // J2: dated by the index, which outlives the RAM tail.
                store
                    .transaction_ref(*tx)
                    .is_some_and(|r| now.saturating_sub(r.posted_at) >= crate::SWEEP_PERIOD_SECS)
            })
            .take(MAX_REPUBLISH_PER_SWEEP)
            .collect()
    };
    for (id, transaction) in due {
        tracing::info!(
            contract = id.0,
            transaction = transaction.0,
            "economy: a public settlement has no chronicle line; announcing it again"
        );
        crate::observer::emit(
            ctx,
            &crate::record::EconomicEvent::ContractChanged {
                id,
                state: crate::record::ContractStateKind::Settled,
                transaction: Some(transaction),
            },
        );
    }
}

/// Page for a live contract that has made **no condition progress at all** for
/// `[economy].stale_contract_age_secs` (E4c verify fold, E9; E6a step 1, DL-08).
///
/// A deadline is optional, and a contract without one that is waiting on evidence
/// nobody will ever produce — a canon edge that was missed, a fact provider that was
/// never wired, an attestor who left the game — waits silently forever with its
/// escrow reserved. Nothing else on the ladder notices: `Draft` has its TTL,
/// `Disputed` has its TTL, and *live and stuck* had neither. Progress is the newest
/// stamp on any condition, or the contract's own creation when none has ever been
/// made; [`page`] dedupes on the subject, so this says it once and renotifies on the
/// ladder's own cadence.
///
/// **Deadline-aware** (DL-08): a contract whose own deadline has not lapsed is never
/// paged for staleness. Its window is the parties' stated patience — a 14-day run is
/// not stuck on day 7 — and the deadline itself is what ends it (`expire`, one step
/// up). A clock the sweep cannot resolve reads as *not lapsed*, the same fail-closed
/// rule the expiry step uses.
fn page_stale_contracts(ctx: &mut Ctx<'_>, now: i64, ic_now: i64) {
    let clocks = Clocks::new(now, ic_now);
    let stale: Vec<(crate::ids::ContractId, i64, bool)> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        let age = store.config().stale_contract_age_secs;
        if age <= 0 {
            return;
        }
        let snap = store.snapshot();
        snap.contracts()
            .filter(|contract| contract.consent.managed.is_none())
            // E6a step 1 verify fold (M2): `Draft` belongs here too. An A′ amendment
            // leaves a draft that may still hold the author's escrow, and one that is
            // never re-offered is money reserved with nobody looking at it — it pages
            // at the stale age, well before the draft TTL unwinds it.
            .filter(|c| {
                matches!(
                    c.state,
                    crate::contract::ContractState::Draft
                        | crate::contract::ContractState::Active
                        | crate::contract::ContractState::ReadyToSettle
                )
            })
            // Doc 18 §4.1 / cut IC6: a live lease ends when somebody says it ends —
            // its one staff attestation, `end_lease`, or a staff cancel. It has no
            // deadline and no condition progress by design, so seven quiet days are not
            // a fault. This exempts housing and recovery leases too, which shared the
            // defect. A `ReadyToSettle` or escrowed `Draft` lease still pages.
            // Cut IC7 (lead note 2b) narrows it to a live lease whose own escrow list
            // is empty: an `Active` lease-shaped contract holding escrow is somebody's
            // money sitting still and pages like any other (`exempt_from_stale_page`).
            .filter(|c| !crate::contract::exempt_from_stale_page(c))
            // A draft with no escrow is not somebody's money sitting still; it is
            // collected on the TTL and says nothing to staff.
            .filter(|c| {
                c.state != crate::contract::ContractState::Draft
                    || !crate::contract::qualifying_escrow(c, snap).holds.is_empty()
            })
            .filter(|c| {
                !c.consent
                    .deadline
                    .is_some_and(|d| !clocks.passed(&d, snap, i64::MIN))
            })
            .filter_map(|c| {
                let progress = c
                    .satisfied
                    .iter()
                    .flatten()
                    .map(|a| a.at)
                    .max()
                    .unwrap_or(c.created_at)
                    .max(c.created_at);
                (now.saturating_sub(progress) > age).then_some((
                    c.id,
                    progress,
                    c.state == crate::contract::ContractState::Draft,
                ))
            })
            .take(MAX_CONTRACTS_PER_SWEEP)
            .collect()
    };
    for (id, progress, draft) in stale {
        let days = now.saturating_sub(progress) / 86_400;
        // M2: a stale escrowed draft is a different sentence and a different set of
        // commands — `@contract/dispute` refuses a draft, and the way out of one is
        // to offer it or cancel it.
        let (context, recommended) = if draft {
            (
                format!(
                    "contract #{id} has sat as a draft for {days} day(s) still holding escrow — an \
                     amendment that was never re-offered; the payer's money is reserved the whole \
                     time and nothing is going to change on its own"
                ),
                vec![
                    format!("@contract/show #{id}"),
                    format!("@contract/offer #{id}"),
                    format!("@contract/cancel #{id}   (releases the escrow)"),
                ],
            )
        } else {
            (
                format!(
                    "contract #{id} has been in force for {days} day(s) with no condition progress \
                     at all; its escrow is reserved the whole time and nothing is going to change \
                     on its own"
                ),
                vec![
                    format!("@contract/show #{id}"),
                    // `@contract/resolve` acts on a *disputed* contract, so the ladder
                    // says how to get there rather than recommending a command that
                    // would refuse.
                    format!("@contract/dispute #{id}"),
                    format!("@contract/resolve #{id}=settle <reason>"),
                    format!("@contract/resolve #{id}=refund <reason>"),
                ],
            )
        };
        page(
            ctx,
            EscalationKind::SettlementRefused,
            EscalationSubject::Contract(id),
            None,
            context,
            recommended,
            now,
        );
    }
}

/// Collect one abandoned `Draft` (§4.3 step 4). Re-checked here against the live
/// store: the read pass is one fire old, and a draft that was offered in between must
/// not be collected out from under its parties.
fn collect_draft(ctx: &mut Ctx<'_>, id: crate::ids::ContractId) {
    // E6a step 1 verify fold (R7): **one predicate.** The read pass selects on
    // `qualifying_escrow(..).holds.is_empty()` and this refused on `c.escrow`, which
    // is an untrusted list — a draft naming a stranger's hold, or one of its own that
    // has since been released, was selected for collection every fire and refused
    // here every fire, burning a per-sweep slot forever and never being collected.
    let collectable = ctx.w.module_state::<EconomyStore>().is_some_and(|s| {
        let snap = s.snapshot();
        snap.contract(id).is_some_and(|c| {
            c.consent.managed.is_none()
                && c.state == crate::contract::ContractState::Draft
                && crate::contract::qualifying_escrow(c, snap).holds.is_empty()
        })
    });
    if !collectable {
        return;
    }
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.snapshot.remove_contract(id);
    }
    tracing::info!(
        contract = id.0,
        "economy: collected an abandoned draft contract"
    );
    api::delete_key(ctx.out, api::contract_key(id));
}

// ---- the escalation ladder ------------------------------------------------

/// What an existing record can do for a fresh page (E4b1 verify fold, S4 + B4c).
enum Reuse {
    /// Its subject already has a record standing; hand back that id untouched.
    Standing(EscalationId),
    /// Its subject has an **acknowledged** record whose cause has come back past the
    /// cool-off: reopen it in place, same id.
    Reopen(EscalationId),
    /// A different subject, but the same **blamed account** inside the escalation
    /// window: name this subject on that record instead of minting a second one.
    Note(EscalationId),
}

/// Open one level-2 record (§4.6) and return its id, or `None` if the store is gone
/// or the allocator is exhausted (B2).
///
/// Three dedupe rules, all of them about the same thing — a staffer should get one
/// alert per decision they actually have to make:
///
/// 1. **One open record per subject.** A refusal that repeats every 30 seconds
///    produces one alert, not a thousand. (E4b1 shipped this one.)
/// 2. **An acknowledged record is REOPENED, never replaced** (S4). Acking without
///    fixing the cause used to mint a fresh id on the very next fire — a new alert
///    number every half minute for one unfixed problem, and the record's own history
///    of how many times it had come back was lost with each one. The same record
///    comes back after [`crate::ESCALATION_RENOTIFY_SECS`], carrying its `reopened`
///    tally; inside the cool-off the subject stays attached to the acked record
///    rather than minting anything.
/// 3. **One record per blamed account** (B4c). One frozen wallet blocks every
///    obligation pointing at it; those are not a dozen decisions, they are one, so
///    the first record names the rest in its context.
// The sweep's standing rule is **no `?` anywhere in this file** (§4.1): a reader
// auditing a tick handler should be able to grep for it and find nothing, without
// having to decide case by case whether a given `?` is an `Option` early-return or an
// error propagation that silently truncates half-done work. Clippy would rather have
// the `?` back here; the discipline is worth more than the line.
#[allow(clippy::question_mark)]
pub(crate) fn page(
    ctx: &mut Ctx<'_>,
    kind: EscalationKind,
    subject: EscalationSubject,
    blamed: Option<AccountId>,
    context: String,
    recommended: Vec<String>,
    now: i64,
) -> Option<EscalationId> {
    let reuse = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return None;
        };
        match store.escalations().find(|e| e.subject == subject) {
            Some(e) if e.is_open() => Some(Reuse::Standing(e.id)),
            Some(e) if e.reopen_due(now) => Some(Reuse::Reopen(e.id)),
            Some(e) => Some(Reuse::Standing(e.id)),
            None => blamed.and_then(|account| {
                store
                    .escalations()
                    .find(|e| {
                        e.is_open()
                            && e.blamed == Some(account)
                            && now.saturating_sub(e.opened_at) <= crate::ESCALATION_WINDOW_SECS
                    })
                    .map(|e| Reuse::Note(e.id))
            }),
        }
    };
    match reuse {
        Some(Reuse::Standing(id)) => return Some(id),
        Some(Reuse::Reopen(id)) => {
            let record = ctx
                .w
                .module_state::<EconomyStore>()
                .and_then(|s| s.escalation(id).cloned());
            if let Some(mut record) = record {
                record.reopen(now);
                tracing::warn!(escalation = id.0, subject = ?subject, reopened = record.reopened, "economy: an acknowledged escalation reopened — its cause never went away");
                let level = record.level;
                api::put_escalation(ctx, record);
                announce_escalation(ctx, id, subject, level);
            }
            return Some(id);
        }
        Some(Reuse::Note(id)) => {
            let record = ctx
                .w
                .module_state::<EconomyStore>()
                .and_then(|s| s.escalation(id).cloned());
            if let Some(mut record) = record {
                record.note_also(&format!(
                    "{} is blocked by the same account",
                    subject.render()
                ));
                tracing::warn!(escalation = id.0, subject = ?subject, "economy: another record is blocked by an already-paged account");
                api::put_escalation(ctx, record);
            }
            return Some(id);
        }
        None => {}
    }
    let id = {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
            return None;
        };
        let Some(id) = store.alloc_escalation() else {
            // B2: at the ceiling the sweep still runs and the subject still stops —
            // it simply stops with a `Staff` reason instead of an alert id.
            tracing::error!(subject = ?subject, "economy: the escalation allocator is exhausted; the subject is suspended WITHOUT an alert record");
            return None;
        };
        id
    };
    let record = Escalation::page(id, kind, subject, context, recommended, now).blaming(blamed);
    tracing::warn!(escalation = id.0, subject = ?subject, context = %record.context, "economy: level-2 escalation opened");
    let level = record.level;
    api::put_escalation(ctx, record);
    let meta = ctx
        .w
        .module_state::<EconomyStore>()
        .map(EconomyStore::meta_doc);
    if let Some(meta) = meta {
        api::persist_meta(ctx.out, meta);
    }
    announce_escalation(ctx, id, subject, level);
    Some(id)
}

/// The most open records one boot or one staff connect re-announces (E6a step 1
/// verify fold, R4 + R5).
pub(crate) const MAX_ANNOUNCE_BACKLOG: usize = 32;

/// **Re-announce the ladder's open records**, newest `limit` first (R4 + R5).
///
/// Two holes it closes, both the same hole seen from opposite ends. Boot opens
/// level-2 records — a quarantined contract, an obligation whose account is gone —
/// and announced *none* of them: only [`page`] announced, and boot does not call it.
/// And a page opened while no staffer is connected reaches nobody, because the
/// audience is whoever happens to be online.
///
/// So: boot announces its reconciliation set once the store is live, and a
/// Wizard-or-better's connect announces the open backlog. The events carry ids and
/// kinds only, exactly as [`announce_escalation`] always has, and the server's
/// observer dedupes on each record's own step — so a staffer hears every record
/// nobody has been told about, and hears nothing twice.
pub(crate) fn announce_open_escalations(ctx: &mut Ctx<'_>, limit: usize) {
    let open: Vec<(EscalationId, EscalationSubject, u8)> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        let mut rows: Vec<(EscalationId, EscalationSubject, u8)> = store
            .escalations()
            .filter(|e| e.is_open())
            .map(|e| (e.id, e.subject, e.level))
            .collect();
        // Newest first for the cap, then oldest first for the reading order.
        rows.sort_by_key(|row| std::cmp::Reverse(row.0));
        rows.truncate(limit);
        rows.reverse();
        rows
    };
    for (id, subject, level) in open {
        announce_escalation(ctx, id, subject, level);
    }
}

/// Tell the observers the ladder opened, reopened or re-notified a record (E6a
/// step 1, DL-05) — the staff-channel fan-out E4b1 deviation 5 left to the server.
/// Ids and kinds only; the record itself is read under the viewer's standing.
fn announce_escalation(ctx: &mut Ctx<'_>, id: EscalationId, subject: EscalationSubject, level: u8) {
    crate::observer::emit(
        ctx,
        &crate::record::EconomicEvent::EscalationOpened { id, subject, level },
    );
}

/// Level 3's companion record (§4.6): the whole-economy freeze is set, so the sweep
/// is doing nothing at all. Opened **once per freeze** — not once per fire, and not
/// once per acknowledgement either (E4b1 verify fold, S4).
///
/// The guard is "any Economy record at all", open or acked, rather than "any *open*
/// one": a staffer who acks the diagnosis has not cleared the freeze (that is
/// `@econ/unfreeze`'s, deliberately), so re-paging them thirty seconds later with a
/// fresh id would punish them for reading it. The record is collected seven days
/// after it is acked, and a freeze still standing then is worth saying again.
fn open_freeze_record(ctx: &mut Ctx<'_>, now: i64) {
    let already = ctx.w.module_state::<EconomyStore>().is_some_and(|s| {
        s.escalations()
            .any(|e| e.subject == EscalationSubject::Economy)
    });
    if already {
        return;
    }
    let Some(id) = ({
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
            return;
        };
        store.alloc_escalation()
    }) else {
        tracing::error!("economy: the escalation allocator is exhausted; the freeze has no record");
        return;
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
    tracing::error!(
        escalation = id.0,
        "economy: the sweep is halted by the whole-economy freeze"
    );
    let level = record.level;
    api::put_escalation(ctx, record);
    let meta = ctx
        .w
        .module_state::<EconomyStore>()
        .map(EconomyStore::meta_doc);
    if let Some(meta) = meta {
        api::persist_meta(ctx.out, meta);
    }
    announce_escalation(ctx, id, EscalationSubject::Economy, level);
}

/// Re-notify about unacknowledged level-2/3 records: first after
/// [`crate::ESCALATION_RENOTIFY_SECS`] (1 h), then daily (§4.6).
///
/// The reminder is a loud log line, a stamp, and — since E6a step 1 (DL-05) — one
/// [`crate::EconomicEvent::EscalationOpened`] per reminder, so the server's staff
/// fan-out follows exactly this schedule and never invents one of its own.
/// `@econ/alerts` is the pull side.
fn renotify(ctx: &mut Ctx<'_>, now: i64) {
    let due: Vec<Escalation> = {
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
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
        let (id, subject, level) = (e.id, e.subject, e.level);
        api::put_escalation(ctx, e);
        announce_escalation(ctx, id, subject, level);
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
        let Some(store) = ctx.w.module_state::<EconomyStore>() else {
            return;
        };
        let snap = store.snapshot();
        let approvals: Vec<_> = snap
            .approvals()
            .filter(|r| {
                matches!(
                    r.status,
                    ApprovalStatus::Expired | ApprovalStatus::Cancelled
                ) && now.saturating_sub(r.expires_at) > crate::APPROVAL_GC_AFTER_SECS
            })
            .take(MAX_GC_PER_SWEEP)
            .map(|r| r.id)
            .collect();
        let holds: Vec<_> = snap
            .holds()
            .filter(|h| !h.status.is_held())
            // These exact terminal holds remain part of the permanent paired
            // purchase audit. Their funding already admitted that footprint.
            .filter(|h| !crate::managed_purchase::closed_namespace(h.holder.namespace.as_str()))
            .filter(|h| !crate::core_integration::bridge::retained_hold(h))
            .filter(|h| !crate::core_refit::bridge::retained_hold(h))
            .filter(|h| h.holder.namespace.as_str() != crate::managed_charter_transfer::NAMESPACE)
            .filter(|h| h.holder.namespace.as_str() != crate::managed_renewal::NAMESPACE)
            .filter(|h| h.holder.namespace.as_str() != crate::managed_house_renewal::NAMESPACE)
            .filter(|h| h.holder.namespace.as_str() != crate::managed_paid_resale::NAMESPACE)
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
                e.acked_at
                    .is_some_and(|at| now.saturating_sub(at) > ESCALATION_GC_AFTER_SECS)
            })
            .take(MAX_GC_PER_SWEEP)
            .map(|e| e.id)
            .collect();
        (approvals, holds, escalations)
    };
    // E6a step 1 verify fold (D4 + R3): the rolling-window index is pruned on **read**
    // as well as on write — an account debited once and never again used to hold its
    // entry forever, and an account whose rule lost its window kept a tail nothing
    // would ever look at. Cheap and unconditional: it walks only the windowed tails.
    if let Some(store) = ctx.w.module_state_mut::<EconomyStore>() {
        store.prune_debit_windows(now);
        // J2: the RAM tail is evicted down to its limit here, never on a post.
        let evicted = store.evict_tail();
        if evicted > 0 {
            tracing::debug!(
                evicted,
                tail = store.tail_len(),
                "economy: RAM tail evicted"
            );
        }
        // Doc 13: cold-cache answers and stale key reads age out here.
        store.age_cold_keys(now);
        // Doc 13 W2 (W-D2): the RAM window — the idempotency map and the
        // transaction index — evicted by age and count, once the backfill is
        // done. The filter and the key documents keep every evicted key.
        let evicted_keys = store.evict_window(now);
        if evicted_keys > 0 {
            tracing::debug!(
                evicted = evicted_keys,
                window = store.window_len(),
                "economy: idempotency window evicted"
            );
        }
    }
    if approvals.is_empty() && holds.is_empty() && escalations.is_empty() {
        return;
    }
    {
        let Some(store) = ctx.w.module_state_mut::<EconomyStore>() else {
            return;
        };
        for id in &approvals {
            store.snapshot.remove_approval(*id);
        }
        for id in &holds {
            store.snapshot.remove_hold(*id);
            store.fold.forget_hold(*id);
            store.hold_expiry_failures.remove(id);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// E4b1 verify fold, S7 — the consecutive-defer ladder's bookkeeping.
    ///
    /// This lives here rather than in `tests/e4b1.rs` because the **increment** side
    /// of an S7 run needs a `Disposition::Transient`, and the only two refusals
    /// classified transient are `JournalUnavailable` and `EconomyFrozen`: the first
    /// has no injection seam in the integration harness (the ledger refuses only on
    /// a sequence/byte overflow that boot itself rejects), and the second makes the
    /// sweep return before it reaches an obligation at all. What *is* testable, and
    /// is the whole of the logic the fold added, is the run itself: it counts
    /// consecutive failures, reports "crossed" exactly once at the cap, saturates
    /// rather than wrapping, and resets when the caller clears the key.
    #[test]
    fn vf_s7_a_transient_defer_run_is_bounded_and_resets() {
        let mut runs: BTreeMap<ObligationId, u32> = BTreeMap::new();
        let id = ObligationId(7);
        // Up to and including the cap: keep retrying, page nobody.
        for expected in 1..=MAX_CONSECUTIVE_DEFERS {
            let (run, exhausted) = bump_run(&mut runs, id, MAX_CONSECUTIVE_DEFERS);
            assert_eq!(run, expected);
            assert!(
                !exhausted,
                "the ladder paged at fire {expected}, inside its own cap"
            );
        }
        // The first fire past it crosses.
        let (run, exhausted) = bump_run(&mut runs, id, MAX_CONSECUTIVE_DEFERS);
        assert_eq!(run, MAX_CONSECUTIVE_DEFERS + 1);
        assert!(exhausted, "the ladder never paged past its cap");

        // Any non-transient outcome clears the run, so "consecutive" means it.
        runs.remove(&id);
        let (run, exhausted) = bump_run(&mut runs, id, MAX_CONSECUTIVE_DEFERS);
        assert_eq!((run, exhausted), (1, false), "the run did not reset");

        // A run that somehow reached the ceiling saturates rather than wrapping to
        // zero — wrapping would silently hand the obligation a fresh allowance.
        runs.insert(id, u32::MAX);
        let (run, exhausted) = bump_run(&mut runs, id, MAX_CONSECUTIVE_DEFERS);
        assert_eq!((run, exhausted), (u32::MAX, true));

        // Runs are per subject: one obligation's deferrals never spend another's.
        let other = ObligationId(8);
        let (run, _) = bump_run(&mut runs, other, MAX_CONSECUTIVE_DEFERS);
        assert_eq!(run, 1);
    }
}

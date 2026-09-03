//! Obligations — the pure recurring-payment state machine (E4a, §3.4).
//!
//! **No money logic lives here.** An obligation says *what is owed, to whom, on
//! which clock, and how much slack there is*; the sweep turns one period into an
//! ordinary [`crate::EconomyPlan`] with the deterministic key
//! [`obligation_key`], posts it through the same `validate`/`apply` path as every
//! other transaction, and feeds the outcome back through [`advance`]. That split is
//! what makes rent, payroll and upkeep untestable-by-inspection *impossible*: the
//! arithmetic here is exercised on hand-built records with no world at all.
//!
//! Three rules from doc 08 §2 and D-E4-14 shape the machine:
//!
//! - **A downtime or time-jump backlog is never charged.** More periods due than
//!   `maximum_catch_up` means the **most recent** `maximum_catch_up` are attempted
//!   and the older excess is [`forgive`]n with a classified [`ForgiveCause`] — the
//!   designed outcome, recorded in `history`, not a wait state.
//! - **A miss is a story state, not a staff page.** A failed period enters
//!   [`ObligationStatus::Grace`]; past `grace.periods` it is `Delinquent`, and
//!   `Delinquent`/`Suspended` leave the due pool entirely, so a long-suspended
//!   obligation cannot starve the others.
//! - **Every period has one deterministic key.** `obligation/{id}/{period}`, derived
//!   from `periods_settled`, so a restart mid-sweep rebuilds the same keys and a
//!   re-post resolves to `AlreadyPosted` rather than double-charging.

use serde::{Deserialize, Serialize};
use smol_str::SmolStr;

use crate::amount::Amount;
use crate::clock::{
    Clocks, EconomicDeadline, EconomicSchedule, SWEEP_GAP_DOWNTIME_SECS, advance as advance_schedule,
};
use crate::ids::{AccountId, ContractId, EscalationId, ObligationId, TransactionId};
use crate::record::{IdempotencyKey, ObligationStatusKind};
use crate::snapshot::EconomySnapshot;
use mush_org::PrincipalId;

/// The bounded audit tail one obligation keeps (§9). The journal is the real
/// history; this is the human-readable summary a `@obligation/show` can print.
pub const MAX_OBLIGATION_HISTORY: usize = 32;
/// The default number of missed periods tolerated before `Delinquent` (§9).
pub const DEFAULT_GRACE_PERIODS: u8 = 1;
/// The largest grace allowance a policy may declare (E4b1 verify fold, S8).
///
/// `missed` is a `u8` that [`advance`] bumps with `saturating_add`, so it stops at
/// 255 — and `missed > periods` is therefore never true at `periods: 255`. A grace of
/// 255 was not "255 periods of slack", it was **infinite** slack: an obligation that
/// could miss forever and never reach `Delinquent`, so it never left the due pool and
/// no consequence hook ever saw it. 254 is the largest allowance the comparison can
/// actually end.
pub const MAX_GRACE_PERIODS: u8 = 254;

/// How many missed periods an obligation tolerates before going `Delinquent`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GracePolicy {
    /// Missed periods tolerated (0 = the first miss is delinquency), ≤
    /// [`MAX_GRACE_PERIODS`].
    pub periods: u8,
}

impl GracePolicy {
    /// Build a policy, **clamping** `periods` to [`MAX_GRACE_PERIODS`] — the
    /// `EconomicSchedule::new` discipline: a stored record can never authorize an
    /// allowance the state machine cannot end (S8).
    #[must_use]
    pub fn new(periods: u8) -> GracePolicy {
        GracePolicy { periods: periods.min(MAX_GRACE_PERIODS) }
    }
    /// The allowance actually applied, clamped on **read** as well as at
    /// construction: a doc hand-written (or written by an older build) with 255 must
    /// not resurrect infinite grace at boot.
    #[must_use]
    pub fn allowance(&self) -> u8 {
        self.periods.min(MAX_GRACE_PERIODS)
    }
}

impl Default for GracePolicy {
    fn default() -> GracePolicy {
        GracePolicy { periods: DEFAULT_GRACE_PERIODS }
    }
}

/// Why an obligation is out of the due pool.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SuspendReason {
    /// A staff act.
    Staff {
        /// Why (≤ [`crate::MAX_REASON_BYTES`]).
        reason: SmolStr,
    },
    /// A level-2 escalation is waiting on a human decision (§4.6).
    Escalated(EscalationId),
}

/// What created this obligation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObligationCause {
    /// A contract's `Obligation` deliverable (a lease, a retainer).
    Contract(ContractId),
    /// A staff act.
    Staff {
        /// Why (≤ [`crate::MAX_REASON_BYTES`]).
        reason: SmolStr,
    },
    /// A ruleset (housing upkeep, career income).
    Ruleset {
        /// The ruleset namespace.
        namespace: SmolStr,
    },
}

/// Why periods were forgiven rather than charged (D-E4-14). Classified from the
/// economy's **own** persisted sweep stamps by [`classify_forgiveness`], never
/// guessed.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum ForgiveCause {
    /// No sweep ran across the missed periods — the server was down.
    Downtime,
    /// The IC clock advanced further than the ratio allows (an epoch or ratio change).
    TimeJump,
    /// Sweeps did run, but this obligation never received post budget.
    Overload,
    /// The periods fell due while the obligation was **out of the due pool** — nobody
    /// was attempting them, so nobody may be charged for them (E4b1 verify fold, S5).
    /// `@obligation/resume` records this for the gap a suspension left behind.
    Suspension,
}

/// Obligation lifecycle. `Delinquent` and `Suspended` are **out of the due pool** —
/// that, not the per-sweep cap, is what stops a dead obligation starving live ones.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObligationStatus {
    /// Settling on schedule.
    Active,
    /// A period was missed and the allowance is not yet spent.
    Grace {
        /// How many periods have been missed in this run.
        missed: u8,
        /// When the allowance ends — **on the schedule's own clock** (C9).
        deadline: EconomicDeadline,
    },
    /// Past the grace allowance: no further attempts; E5's consequence hook owns it.
    Delinquent,
    /// Out of the due pool pending a staff act or an escalation.
    Suspended {
        /// Why.
        reason: SuspendReason,
    },
    /// A one-shot that settled.
    Satisfied,
    /// Cancelled.
    Cancelled,
}

impl ObligationStatus {
    /// The payload-free discriminant an [`crate::EconomicEvent`] may carry.
    #[must_use]
    pub fn kind(&self) -> ObligationStatusKind {
        match self {
            ObligationStatus::Active => ObligationStatusKind::Active,
            ObligationStatus::Grace { .. } => ObligationStatusKind::Grace,
            ObligationStatus::Delinquent => ObligationStatusKind::Delinquent,
            ObligationStatus::Suspended { .. } => ObligationStatusKind::Suspended,
            ObligationStatus::Satisfied => ObligationStatusKind::Satisfied,
            ObligationStatus::Cancelled => ObligationStatusKind::Cancelled,
        }
    }
    /// Whether this obligation is in the **due pool** — the only two statuses the
    /// sweep will attempt (D-E4-3).
    #[must_use]
    pub fn in_due_pool(&self) -> bool {
        matches!(self, ObligationStatus::Active | ObligationStatus::Grace { .. })
    }
}

/// One entry of the bounded audit tail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObligationEvent {
    /// A period settled.
    Settled {
        /// Which period index.
        period: u32,
        /// When (on the schedule's clock).
        at: i64,
        /// The transaction that carried it, when known.
        transaction: Option<TransactionId>,
    },
    /// A period was attempted and failed.
    Missed {
        /// Which period index.
        period: u32,
        /// When.
        at: i64,
    },
    /// Periods were forgiven rather than charged (D-E4-14).
    Forgiven {
        /// How many.
        periods: u32,
        /// The first forgiven period index (inclusive).
        from: u32,
        /// The last forgiven period index (exclusive).
        to: u32,
        /// Why.
        cause: ForgiveCause,
        /// When.
        at: i64,
    },
    /// A level-2 escalation was opened for this obligation.
    Escalated {
        /// The escalation record.
        id: EscalationId,
        /// When.
        at: i64,
    },
    /// A suspended obligation returned to the due pool.
    Resumed {
        /// When.
        at: i64,
    },
}

/// A recurring debt: a fixed amount per period, on a named clock, with bounded
/// catch-up and a grace allowance.
///
/// `amount` is a fixed [`Amount`] in E4; lifestyle-tier and percentage amounts are an
/// E5 `ObligationAmount` variant, which is why nothing here reads the payer's balance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Obligation {
    /// Identity (never reused).
    pub id: ObligationId,
    /// Who owes.
    pub debtor: PrincipalId,
    /// Who is owed.
    pub creditor: PrincipalId,
    /// The account debited each period.
    pub debit_account: AccountId,
    /// The account credited each period.
    pub credit_account: AccountId,
    /// How much, per period.
    pub amount: Amount,
    /// When and how often.
    pub schedule: EconomicSchedule,
    /// How much slack before delinquency.
    pub grace: GracePolicy,
    /// Whether a routine success notifies (default `false` — §14.3: routine
    /// settlement is silent, only transitions speak).
    pub notify_on_success: bool,
    /// Lifecycle.
    pub status: ObligationStatus,
    /// What created it.
    pub cause: ObligationCause,
    /// The registered faucet this obligation is allowed to **mint** from, if any
    /// (E4b1 verify fold, M1).
    ///
    /// A `System` debit account creates money out of nothing every period, so it is
    /// refused everywhere by default: `@obligation/create` will not build one, and
    /// the sweep's `ObligationAuth` refuses one at every settlement. The single way
    /// in is [`crate::create_system_obligation`], the ruleset's own mint entry, which
    /// stamps the faucet here — and even then the per-period amount is re-checked
    /// against the live [`crate::EconomyConfig`] cap at every settlement, so a doc
    /// edited above the cap after creation is paged, not paid.
    ///
    /// `None` on every record written before M1, and on every record a command makes.
    #[serde(default)]
    pub minted_by: Option<PrincipalId>,
    /// How many periods have been **closed** — settled, missed, or forgiven. It is
    /// the index base for the next period's deterministic key
    /// (`obligation/{id}/{periods_settled}`), so a missed period closes too: were it
    /// not to, the next sweep would re-derive the same key and the grace counter
    /// would run out inside one sweep period rather than over the schedule.
    pub periods_settled: u32,
    /// The bounded audit tail (≤ [`MAX_OBLIGATION_HISTORY`]); the journal is the
    /// real history.
    pub history: Vec<ObligationEvent>,
    /// How many periods have **settled**, ever.
    ///
    /// `history` is a FIFO of 32, so the tally it used to be read from evaporates on
    /// the 33rd event — and "has this tenant ever paid?" is exactly the question a
    /// landlord asks after a long run (E4a verify fold, finding 14d). These three
    /// counters are the durable tally; the tail stays the human-readable detail.
    #[serde(default)]
    pub settled_total: u32,
    /// How many periods have been **missed**, ever.
    #[serde(default)]
    pub missed_total: u32,
    /// How many periods have been **forgiven**, ever (D-E4-14).
    #[serde(default)]
    pub forgiven_total: u32,
    /// Staleness stamp.
    pub revision: u64,
}

/// The deterministic idempotency key for one period of one obligation:
/// `obligation/{id}/{period}` (§4.3). A restart mid-sweep re-derives it exactly, so
/// the re-post resolves to `AlreadyPosted` instead of charging twice.
#[must_use]
pub fn obligation_key(id: ObligationId, period: u32) -> IdempotencyKey {
    IdempotencyKey::new(SmolStr::new_static("obligation"), SmolStr::new(format!("{}/{}", id.0, period)))
}

/// Why an obligation transition was refused.
#[derive(Copy, Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ObligationError {
    /// An outcome arrived for a period **ahead** of the ones already closed (E4a
    /// verify fold, finding 9). Folding it would leave a hole in the period index —
    /// the deterministic key base — and would silently skip the forgiveness step
    /// [`plan_sweep`] asked for before those attempts.
    #[error("that settlement outcome is out of order")]
    OutOfOrder,
}

/// What one sweep step should do with one obligation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SweepIntent {
    /// The period indexes to attempt, oldest first — never more than the schedule's
    /// (already-clamped) `maximum_catch_up`.
    pub attempt: Vec<u32>,
    /// How many older periods to forgive before attempting those.
    pub forgive: u32,
    /// The first forgiven period index (inclusive).
    pub forgive_from: u32,
    /// The last forgiven period index (exclusive).
    pub forgive_to: u32,
    /// The period counter cannot advance any further without overflowing `u32`, so
    /// `forgive` periods are forgiven **wholesale**: no index is consumed, no period
    /// is attempted, and the schedule is re-anchored past `now` by [`forgive_all`]
    /// (E4a verify fold, finding 13). `forgive_from == forgive_to` whenever this is
    /// set, because nothing is being indexed.
    pub reanchor: bool,
}

impl SweepIntent {
    /// Whether this obligation wants nothing from this sweep.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.attempt.is_empty() && self.forgive == 0
    }
}

/// The pure sweep step for one obligation at `now` **on its own clock**.
///
/// An obligation outside the due pool yields an empty intent. Otherwise: if at most
/// `maximum_catch_up` periods are due they are all attempted; if more are due, the
/// **most recent** `maximum_catch_up` are attempted and the older excess is reported
/// for forgiveness (D-E4-14) — doc 08 §2 forbids charging a backlog, so this is the
/// designed outcome and not a fallback.
#[must_use]
pub fn plan_sweep(o: &Obligation, now: i64) -> SweepIntent {
    if !o.status.in_due_pool() {
        return SweepIntent::default();
    }
    let due = crate::clock::due_periods(&o.schedule, now);
    if due == 0 {
        return SweepIntent::default();
    }
    let cap = u32::from(o.schedule.maximum_catch_up.clamp(1, crate::clock::MAX_CATCH_UP_CAP));
    let base = o.periods_settled;
    // The period index is the deterministic KEY base, so it may never be saturated
    // into: `base.saturating_add(..)` at the ceiling hands several periods the same
    // `obligation/{id}/{period}` key, and the second post of the sweep then resolves
    // to `AlreadyPosted` against the first — a period silently paid by another
    // period's transaction. When the counter cannot advance, forgive everything
    // instead and re-anchor the schedule (finding 13).
    if base.checked_add(due).is_none() || base.checked_add(cap).is_none() {
        return SweepIntent {
            attempt: Vec::new(),
            forgive: due,
            forgive_from: base,
            forgive_to: base,
            reanchor: true,
        };
    }
    if due <= cap {
        return SweepIntent {
            attempt: (0..due).map(|k| base + k).collect(),
            forgive: 0,
            forgive_from: base,
            forgive_to: base,
            reanchor: false,
        };
    }
    let forgive = due - cap;
    let from = base;
    let to = base + forgive;
    SweepIntent {
        attempt: (0..cap).map(|k| to + k).collect(),
        forgive,
        forgive_from: from,
        forgive_to: to,
        reanchor: false,
    }
}

/// The outcome of one period's settlement post, as the sweep saw it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SettleOutcome {
    /// A new transaction posted.
    Posted(TransactionId),
    /// The deterministic key already resolved (a duplicate fire, or a restart).
    AlreadyPosted(TransactionId),
    /// The debtor did not have the available funds.
    InsufficientAvailable,
    /// Refused for any other reason (authorization, a frozen account, a cap).
    Refused,
}

impl SettleOutcome {
    /// Whether the period is paid.
    #[must_use]
    pub fn is_settled(&self) -> bool {
        matches!(self, SettleOutcome::Posted(_) | SettleOutcome::AlreadyPosted(_))
    }
}

fn push_event(o: &mut Obligation, event: ObligationEvent) {
    if o.history.len() >= MAX_OBLIGATION_HISTORY {
        o.history.remove(0);
    }
    o.history.push(event);
}

/// Fold one period's settlement outcome into the obligation.
///
/// Pure, and **exactly one period at a time** (E4a verify fold, finding 9):
///
/// - `period < periods_settled` — already closed: a duplicate, `Ok(unchanged)`, so a
///   re-delivered sweep result can never move a status twice.
/// - `period > periods_settled` — [`ObligationError::OutOfOrder`]. Folding it would
///   leave a hole in the key index, and it is also the gate that keeps
///   [`plan_sweep`]'s contract honest: the intent's `attempt` list starts *past* the
///   periods it asked to have forgiven, so a caller who skips [`forgive`] and posts
///   the attempts straight away is refused instead of silently swallowing them.
/// - `period == periods_settled` — closed by this outcome. A settled outcome records
///   [`ObligationEvent::Settled`] **whatever the status**: money did move, and
///   dropping the record because the obligation had gone `Delinquent` would lose a
///   real payment. The *status* is only recomputed for an obligation in the due pool,
///   so a `Delinquent`/`Suspended` one keeps its state until E4b1's `reinstate` (or
///   E5's hook) moves it deliberately.
pub fn advance(
    o: &Obligation,
    period: u32,
    outcome: SettleOutcome,
    now: i64,
) -> Result<Obligation, ObligationError> {
    if period < o.periods_settled {
        return Ok(o.clone());
    }
    if period > o.periods_settled {
        return Err(ObligationError::OutOfOrder);
    }
    // A miss on an obligation already out of the due pool changes nothing: it was
    // never attempted, so there is nothing to record.
    if !outcome.is_settled() && !o.status.in_due_pool() {
        return Ok(o.clone());
    }
    let mut next = o.clone();
    next.schedule = advance_schedule(&o.schedule, 1);
    next.periods_settled = period.saturating_add(1);
    next.revision += 1;

    if outcome.is_settled() {
        let transaction = match outcome {
            SettleOutcome::Posted(t) | SettleOutcome::AlreadyPosted(t) => Some(t),
            _ => None,
        };
        push_event(&mut next, ObligationEvent::Settled { period, at: now, transaction });
        next.settled_total = next.settled_total.saturating_add(1);
        // A settled period clears the grace run and a one-shot is finished — but
        // only for an obligation the sweep was actually attending to.
        if o.status.in_due_pool() {
            next.status = if o.schedule.cadence.is_none() {
                ObligationStatus::Satisfied
            } else {
                ObligationStatus::Active
            };
        }
        return Ok(next);
    }

    push_event(&mut next, ObligationEvent::Missed { period, at: now });
    next.missed_total = next.missed_total.saturating_add(1);
    let missed = match &o.status {
        ObligationStatus::Grace { missed, .. } => missed.saturating_add(1),
        _ => 1,
    };
    // A one-shot has no NEXT period to grace into: its schedule is finished
    // (`next_due == i64::MAX`), so a `Grace` deadline there would be an allowance
    // that never expires and an obligation that never resolves. It goes straight to
    // `Delinquent` and out of the due pool (finding 11).
    let past_grace = u32::from(missed) > u32::from(o.grace.allowance());
    next.status = if o.schedule.cadence.is_none() || past_grace {
        ObligationStatus::Delinquent
    } else {
        ObligationStatus::Grace {
            missed,
            // The allowance runs to the next due instant, expressed on the
            // SCHEDULE's clock — never wall time compared against an IC schedule.
            deadline: EconomicDeadline { clock: next.schedule.clock, at: next.schedule.next_due },
        }
    };
    Ok(next)
}

/// Forgive the periods `[from, to)` — advance past them without charging, and record
/// why (D-E4-14). The counterpart of [`plan_sweep`]'s `forgive` half.
#[must_use]
pub fn forgive(o: &Obligation, from: u32, to: u32, cause: ForgiveCause, now: i64) -> Obligation {
    if to <= from || from < o.periods_settled {
        return o.clone();
    }
    let mut next = o.clone();
    let periods = to - from;
    next.schedule = advance_schedule(&o.schedule, periods);
    next.periods_settled = to;
    next.forgiven_total = next.forgiven_total.saturating_add(periods);
    next.revision += 1;
    push_event(&mut next, ObligationEvent::Forgiven { periods, from, to, cause, at: now });
    next
}

/// Forgive `periods` **wholesale**, consuming no period index at all: the schedule
/// is re-anchored to the first due instant after `now` and `periods_settled` stays
/// exactly where it was.
///
/// The counterpart of [`SweepIntent::reanchor`] (E4a verify fold, finding 13). It
/// exists because the period index is the deterministic key base and is a `u32`: an
/// obligation whose counter is at the ceiling cannot advance without handing two
/// periods the same `obligation/{id}/{period}` key. Forgiving without indexing is the
/// only safe answer, and doc 08 §2 already says a backlog is never charged.
#[must_use]
pub fn forgive_all(o: &Obligation, periods: u32, cause: ForgiveCause, now: i64) -> Obligation {
    let mut next = o.clone();
    next.schedule = crate::clock::reanchor(&o.schedule, now);
    next.forgiven_total = next.forgiven_total.saturating_add(periods);
    next.revision += 1;
    let index = o.periods_settled;
    push_event(&mut next, ObligationEvent::Forgiven {
        periods,
        // Nothing was indexed, so the span is empty — the `periods` count is the
        // honest record of how much was let go.
        from: index,
        to: index,
        cause,
        at: now,
    });
    next
}

/// Classify why periods went unsettled, from the economy's **own** persisted sweep
/// stamps (`meta.last_sweep_real` / `meta.last_sweep_ic`) — never from a guess.
///
/// `ratio_hint` is the ruleset's expected IC seconds per real second (≥ 1);
/// `spans_boot` says whether a **boot happened inside the gap** — the caller reads
/// that off `meta.last_boot_real` against `meta.last_sweep_real`, and it is the one
/// piece of evidence that is not a guess about elapsed time.
///
/// The order matters: an IC clock that outran the ratio is a
/// [`ForgiveCause::TimeJump`] (an epoch or ratio change) even across a restart,
/// because a plain restart advances both clocks together; a gap the economy can
/// **prove** contains a restart is [`ForgiveCause::Downtime`] however short it was
/// (E4b1 verify fold, S3 — a 90-second restart is downtime, not a starved post
/// budget); so is a long gap with no such marker; and anything else means sweeps
/// *did* run and this obligation simply never got budget —
/// [`ForgiveCause::Overload`], the one cause that escalates when it repeats (§4.6).
///
/// Misreading a restart as `Overload` is the expensive direction: two ordinary
/// restarts inside [`crate::ESCALATION_WINDOW_SECS`] would suspend a healthy
/// obligation and page a staffer about a problem that never existed.
#[must_use]
pub fn classify_forgiveness(
    last_sweep_real: i64,
    last_sweep_ic: i64,
    now_real: i64,
    now_ic: i64,
    ratio_hint: i64,
    spans_boot: bool,
) -> ForgiveCause {
    // A sweep stamp in the FUTURE means the wall clock was rewound (an NTP step, a
    // container clock, a restored backup). That is a time jump by definition, and
    // clamping the elapsed span to zero would have read it as `Overload` — the one
    // cause that escalates on repeat, so a rewound clock would page a human about a
    // starved post budget that never existed (E4a verify fold, finding 14c).
    if last_sweep_real > now_real {
        return ForgiveCause::TimeJump;
    }
    let real_elapsed = i128::from(now_real).saturating_sub(i128::from(last_sweep_real)).max(0);
    let ic_elapsed = i128::from(now_ic).saturating_sub(i128::from(last_sweep_ic)).max(0);
    let ratio = i128::from(ratio_hint.max(1));
    // One sweep period of slack, so ordinary rounding never reads as a jump.
    let allowed = real_elapsed * ratio + ratio * i128::from(SWEEP_GAP_DOWNTIME_SECS);
    if ic_elapsed > allowed {
        return ForgiveCause::TimeJump;
    }
    // The boot marker beats the clock arithmetic: a gap that provably contains a
    // restart is downtime at ANY length (S3).
    if spans_boot || real_elapsed > i128::from(SWEEP_GAP_DOWNTIME_SECS) {
        return ForgiveCause::Downtime;
    }
    ForgiveCause::Overload
}

/// The obligation's own due count, resolved on its clock through `clocks`
/// (a cadence-clock schedule reads its position from the snapshot).
#[must_use]
pub fn due_now(o: &Obligation, clocks: &Clocks, snapshot: &EconomySnapshot) -> u32 {
    clocks.due_periods(&o.schedule, snapshot)
}

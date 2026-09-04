//! The tenancy state machine (E5a, 08d §3.2) — **D-E5-1: a home never disappears on
//! its own.**
//!
//! ```text
//! Pending ── landlord + tenant accept ──▶ Active                       (Obj.home set for each tenant)
//! Active ── period Unsupported / rent missed ──▶ Grace{since}          (one private notice)
//! Grace ── supported again / paid ──▶ Active                           (silent)
//! Grace ── GRACE_PERIODS elapsed ──▶ Delinquent{since}                 (one private notice; landlord notice)
//! Delinquent ── supported again / arrears paid ──▶ Active              (silent)
//! Delinquent ── DELINQUENT_PERIODS elapsed ──▶ EvictionPending{window} (one private notice; job; optional scene)
//! EvictionPending ── supported again / paid ──▶ Active                 (the window closes)
//! EvictionPending ── explicit resolution ──▶ Ended{how}                (tenant, staff, or contract settlement)
//! EvictionPending ── window elapses ──▶ EvictionPending                (decaying re-notify; escalation; NO state change)
//! ```
//!
//! Three properties this module is built to make **unwritable**, not merely untested:
//!
//! - **No path to `Ended` without a [`Resolution`].** The enum variant takes one, and
//!   [`resolve`] is the only function that produces it. Automatic processing —
//!   [`advance`], [`window_elapsed`], every `*_elapsed` arrow — cannot reach it at all.
//! - **No clock is read here.** Every function is deterministic in `now`, a position
//!   on the tenancy's own [`mush_economy::CadenceId`] clock (D-E5-14), plus the period
//!   counts stored in the status. A clock that moved backwards yields zero elapsed
//!   periods (the `due_periods` discipline), never a wrapped or negative count.
//! - **Every forbidden arrow is a typed error**, so "not yet" and "never" are
//!   different answers.
//!
//! The story window is bounded in **notices**, not in duration (owner answer 8): an
//! offline player's home persists indefinitely, and decision 25's ladder does the
//! paging.

use std::collections::BTreeSet;

use mush_core::ObjId;
use mush_economy::EscalationId;

use crate::assess::Assessment;
use crate::error::HousingError;
use crate::ids::SceneRef;
use crate::record::{Resolution, StoryWindow, Tenancy, TenancyEvent, TenancyStatus};

// ---------------------------------------------------------------------------
// Constants (08d §9)
// ---------------------------------------------------------------------------

/// Periods in `Grace` before `Delinquent` (§9, §11 answer 1).
pub const GRACE_PERIODS: i64 = 1;
/// Periods in `Delinquent` before the story window opens (§9, §11 answer 1).
///
/// With [`GRACE_PERIODS`] that is **three periods** from the first unsupported period
/// to the window: miss at `p` → `Grace`, `p+1` → `Delinquent`, `p+3` →
/// `EvictionPending`.
pub const DELINQUENT_PERIODS: i64 = 2;
/// Periods between story-window re-notices, while any remain (§9).
pub const WINDOW_RENOTIFY: i64 = 1;
/// The most re-notices one story window sends before going quiet (§9).
pub const MAX_WINDOW_NOTICES: u8 = 3;
/// Periods of **silence after the last re-notice** before the level-2 escalation
/// opens (§9).
///
/// 08d §9 lists this as "`WINDOW_ESCALATE` 2 periods" alongside "`MAX_WINDOW_NOTICES`
/// 3, then silent until escalation". Read literally as "2 periods after the window
/// opened" the two constants contradict each other — the escalation would fire at
/// period 2 while the third re-notice is still due at period 3, and §3.2's "then
/// **nothing further** until the level-2 escalation opens" would describe a quiet gap
/// that cannot exist. Measuring it from the **last notice** is the only reading under
/// which both numbers hold, so that is what this is: with a window opened at `p`, the
/// re-notices land at `p+1`, `p+2`, `p+3` and the escalation at `p+5`.
pub const WINDOW_ESCALATE: i64 = 2;
/// Periods between re-notices **after** the escalation is open (§9: "then ≤ weekly").
///
/// "Weekly" is wall time and this library reads no wall clock, so the bound is
/// expressed on the tenancy's own cadence: seven periods is **at least** a week on
/// any cadence of a day or longer, which is every cadence a housing assessment is
/// meant to run on (a cadence finer than daily would make the whole three-period
/// ladder finish inside three days). E5b re-checks that when it registers the clock.
pub const WINDOW_ESCALATED_RENOTIFY: i64 = 7;

// ---------------------------------------------------------------------------
// Period arithmetic
// ---------------------------------------------------------------------------

/// Periods elapsed from `since` to `now` on the tenancy's own cadence.
///
/// `0` when the clock did not move forward — an epoch change, a re-pointed cadence,
/// or a corrupt stamp in the future never yields a negative or wrapped count (the
/// `mush_economy::due_periods` rule). Checked, never saturating: two positions a
/// corrupt document could hold are `i64::MIN` and `i64::MAX`, whose difference does
/// not fit, and silently saturating that to "a very long time" would fire every
/// elapse arrow at once.
pub fn periods_since(since: i64, now: i64) -> Result<i64, HousingError> {
    if now <= since {
        return Ok(0);
    }
    now.checked_sub(since).ok_or(HousingError::Overflow)
}

// ---------------------------------------------------------------------------
// The arrows
// ---------------------------------------------------------------------------

fn move_to(t: &Tenancy, to: TenancyStatus, now: i64) -> Tenancy {
    let mut next = t.clone();
    let from = t.status.kind();
    let to_kind = to.kind();
    next.status = to;
    next.push_event(TenancyEvent::StatusChanged { from, to: to_kind, at: now });
    next.revision = next.revision.saturating_add(1);
    next
}

/// `Pending ── landlord + tenant accept ──▶ Active`.
///
/// **Both** sides, in one arrow: the landlord's consent plus every named tenant's.
/// A shared lease is not half-accepted — a co-tenant who never agreed would be homed
/// on somebody else's word and would be assessed against a tier they never chose — so
/// a missing acceptance is [`HousingError::NotAccepted`], not a partially-live
/// tenancy.
///
/// On success the tenancy is `Active` and `homed` records every tenant whose
/// `Obj.home` E5b is about to point at the property's anchor
/// ([`crate::homed_on_activate`], D-E5-11).
pub fn accept(
    t: &Tenancy,
    landlord_accepted: bool,
    tenant_accepted: &BTreeSet<ObjId>,
    now: i64,
) -> Result<Tenancy, HousingError> {
    if t.status.is_ended() {
        return Err(HousingError::AlreadyEnded(t.id));
    }
    if !matches!(t.status, TenancyStatus::Pending) {
        return Err(HousingError::State(t.id));
    }
    if !landlord_accepted {
        return Err(HousingError::NotAccepted);
    }
    if !t.tenants.iter().all(|who| tenant_accepted.contains(who)) {
        return Err(HousingError::NotAccepted);
    }
    let mut next = move_to(t, TenancyStatus::Active, now);
    next.homed = crate::homed_on_activate(t);
    next.push_event(TenancyEvent::Accepted { at: now });
    Ok(next)
}

/// `Active ── period Unsupported / rent missed ──▶ Grace{since}`.
///
/// A miss on a tenancy **already** in the ladder changes nothing: the run started
/// when it started, and re-stamping `since` on every unsupported period would reset
/// the clock and mean the tenancy never reached `Delinquent`. The elapse arrows
/// ([`grace_elapsed`], [`delinquent_elapsed`]) are what move it on.
pub fn period_missed(t: &Tenancy, now: i64) -> Result<Tenancy, HousingError> {
    if t.status.is_ended() {
        return Err(HousingError::AlreadyEnded(t.id));
    }
    match t.status {
        TenancyStatus::Pending => Err(HousingError::State(t.id)),
        TenancyStatus::Active => Ok(move_to(t, TenancyStatus::Grace { since: now }, now)),
        _ => Ok(t.clone()),
    }
}

/// `Grace | Delinquent | EvictionPending ── supported again / paid ──▶ Active` —
/// silently, at **any** stage (§3.2).
///
/// Recovery from `EvictionPending` closes the window: the caller reads the outgoing
/// window off the tenancy it passed in (its job id is what E5b resolves "recovered").
/// A tenancy that is already `Active` is unchanged, with no revision bump — routine
/// supported periods post nothing and say nothing (§14.3).
pub fn period_supported(t: &Tenancy, now: i64) -> Result<Tenancy, HousingError> {
    if t.status.is_ended() {
        return Err(HousingError::AlreadyEnded(t.id));
    }
    match t.status {
        TenancyStatus::Pending => Err(HousingError::State(t.id)),
        TenancyStatus::Active => Ok(t.clone()),
        _ => Ok(move_to(t, TenancyStatus::Active, now)),
    }
}

/// `Grace ── GRACE_PERIODS elapsed ──▶ Delinquent{since}`.
///
/// [`HousingError::State`] off `Grace`; [`HousingError::NotDue`] before the allowance
/// is spent.
pub fn grace_elapsed(t: &Tenancy, now: i64) -> Result<Tenancy, HousingError> {
    if t.status.is_ended() {
        return Err(HousingError::AlreadyEnded(t.id));
    }
    let TenancyStatus::Grace { since } = t.status else {
        return Err(HousingError::State(t.id));
    };
    if periods_since(since, now)? < GRACE_PERIODS {
        return Err(HousingError::NotDue);
    }
    Ok(move_to(t, TenancyStatus::Delinquent { since: now }, now))
}

/// `Delinquent ── DELINQUENT_PERIODS elapsed ──▶ EvictionPending{window}` — the
/// **window opens** here, and this is where automatic processing stops for good.
///
/// Nothing past this point moves the tenancy without a human: [`window_elapsed`]
/// only counts notices, and [`resolve`] needs a [`Resolution`] somebody chose.
pub fn delinquent_elapsed(t: &Tenancy, now: i64) -> Result<Tenancy, HousingError> {
    if t.status.is_ended() {
        return Err(HousingError::AlreadyEnded(t.id));
    }
    let TenancyStatus::Delinquent { since } = t.status else {
        return Err(HousingError::State(t.id));
    };
    if periods_since(since, now)? < DELINQUENT_PERIODS {
        return Err(HousingError::NotDue);
    }
    Ok(move_to(t, TenancyStatus::EvictionPending { window: StoryWindow::opened_at(now) }, now))
}

/// Record the `wod-story` job (and the optional scene) E5b opened for the window.
///
/// Separate from [`delinquent_elapsed`] because the job id does not exist until the
/// module has actually opened one, and because a scene is started later, when the ST
/// asks for it through the job.
pub fn attach_window(
    t: &Tenancy,
    job: Option<u64>,
    scene: Option<SceneRef>,
) -> Result<Tenancy, HousingError> {
    if t.status.is_ended() {
        return Err(HousingError::AlreadyEnded(t.id));
    }
    let TenancyStatus::EvictionPending { window } = t.status.clone() else {
        return Err(HousingError::NoWindow(t.id));
    };
    let mut w = window;
    if job.is_some() {
        w.job = job;
    }
    if scene.is_some() {
        w.scene = scene;
    }
    let mut next = t.clone();
    next.status = TenancyStatus::EvictionPending { window: w };
    next.revision = next.revision.saturating_add(1);
    Ok(next)
}

/// What the story window wants at `now` — the **decay schedule**, as a value.
///
/// Split out from [`window_elapsed`] because the escalation record is minted by E5b:
/// the module asks what is due, opens an [`EscalationId`] only when the answer is
/// [`WindowAction::Escalate`], and folds the result back through [`window_elapsed`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum WindowAction {
    /// Nothing is due — the common answer, and the whole point of the decay: a
    /// window that has spent its notices and not yet reached the escalation gap is
    /// **silent**, however many periods elapse.
    #[default]
    Silent,
    /// Send a re-notice (there are still some left).
    Notify,
    /// Open the level-2 escalation, carrying the same recommended commands (§4.6).
    Escalate,
    /// Re-notice at the post-escalation rate ([`WINDOW_ESCALATED_RENOTIFY`]).
    NotifyEscalated,
}

/// What [`window_elapsed`] would do at `now`. [`WindowAction::Silent`] for every
/// tenancy that is not `EvictionPending`, so a caller can ask unconditionally.
#[must_use]
pub fn window_due(t: &Tenancy, now: i64) -> WindowAction {
    let TenancyStatus::EvictionPending { window } = &t.status else {
        return WindowAction::Silent;
    };
    // A clock that moved backwards yields zero elapsed periods, so nothing is due —
    // never a burst of notices catching up on a rewound cadence.
    let Ok(since_notice) = periods_since(window.last_notice, now) else {
        return WindowAction::Silent;
    };
    if window.escalation.is_some() {
        if since_notice >= WINDOW_ESCALATED_RENOTIFY {
            return WindowAction::NotifyEscalated;
        }
        return WindowAction::Silent;
    }
    if window.notified < MAX_WINDOW_NOTICES {
        if since_notice >= WINDOW_RENOTIFY {
            return WindowAction::Notify;
        }
        return WindowAction::Silent;
    }
    if since_notice >= WINDOW_ESCALATE {
        return WindowAction::Escalate;
    }
    WindowAction::Silent
}

/// `EvictionPending ── window elapses ──▶ EvictionPending` — **no state change**,
/// ever (§3.2).
///
/// The only things that move are the notice count, the notice stamp, and (once) the
/// escalation id. `notified` is capped at [`MAX_WINDOW_NOTICES`] and never counts
/// past it, so the post-escalation notices do not inflate it.
///
/// `escalation` must be `Some` exactly when [`window_due`] answers
/// [`WindowAction::Escalate`]; supplying nothing there is
/// [`HousingError::NeedsEscalation`] rather than a silently skipped page.
pub fn window_elapsed(
    t: &Tenancy,
    now: i64,
    escalation: Option<EscalationId>,
) -> Result<Tenancy, HousingError> {
    if t.status.is_ended() {
        return Err(HousingError::AlreadyEnded(t.id));
    }
    let TenancyStatus::EvictionPending { window } = t.status.clone() else {
        return Err(HousingError::NoWindow(t.id));
    };
    let action = window_due(t, now);
    if action == WindowAction::Silent {
        return Ok(t.clone());
    }
    let mut w = window;
    let mut next = t.clone();
    match action {
        WindowAction::Notify => {
            w.notified = w.notified.saturating_add(1).min(MAX_WINDOW_NOTICES);
            w.last_notice = now;
            next.push_event(TenancyEvent::Notified { count: w.notified, at: now });
        }
        WindowAction::Escalate => {
            let Some(id) = escalation else {
                return Err(HousingError::NeedsEscalation);
            };
            w.escalation = Some(id);
            w.last_notice = now;
            next.push_event(TenancyEvent::Escalated { id, at: now });
        }
        WindowAction::NotifyEscalated => {
            w.last_notice = now;
            next.push_event(TenancyEvent::Notified { count: w.notified, at: now });
        }
        WindowAction::Silent => return Ok(t.clone()),
    }
    next.status = TenancyStatus::EvictionPending { window: w };
    next.revision = next.revision.saturating_add(1);
    Ok(next)
}

/// `── explicit resolution ──▶ Ended{how}`: the **only** way a tenancy ends
/// (D-E5-1).
///
/// A tenant's own move-out, a staff resolution after a scene, a deliberate staff
/// waiver, or a contract settlement. Allowed from every live status **and** from
/// `Pending` (a declined or withdrawn offer is an ending somebody chose, which is the
/// rule this decision is about); refused on an already-ended tenancy, so a second
/// resolution cannot overwrite the first one's record of what happened.
///
/// The caller clears `Obj.home` for [`crate::to_unhome_on_end`]'s answer in the same
/// breath — that is the other half of D-E5-11.
pub fn resolve(t: &Tenancy, how: Resolution, now: i64) -> Result<Tenancy, HousingError> {
    if t.status.is_ended() {
        return Err(HousingError::AlreadyEnded(t.id));
    }
    if let Resolution::StaffEnded { reason, .. } = &how
        && reason.len() > crate::MAX_REASON_BYTES
    {
        return Err(HousingError::TooLarge("resolution reason"));
    }
    let mut next = move_to(t, TenancyStatus::Ended { how: how.clone() }, now);
    next.push_event(TenancyEvent::Ended { how, at: now });
    Ok(next)
}

// ---------------------------------------------------------------------------
// The composed ladder step
// ---------------------------------------------------------------------------

/// Fold one assessed period into the tenancy — the step E5b's cadence tick runs.
///
/// It is a **composition of the arrows above and nothing else**, so the ladder lives
/// in one tested place instead of being re-derived by the module:
///
/// | status | `Supported`/`Strained` | `Unsupported` | `Unknown` |
/// |---|---|---|---|
/// | `Active` | unchanged | → `Grace{now}` | unchanged |
/// | `Grace{since}` | → `Active` | → `Delinquent{now}` once [`GRACE_PERIODS`] elapsed, else unchanged | unchanged |
/// | `Delinquent{since}` | → `Active` | → `EvictionPending` once [`DELINQUENT_PERIODS`] elapsed, else unchanged | unchanged |
/// | `EvictionPending` | → `Active` (window closes) | unchanged — see below | unchanged |
///
/// Two deliberate holes:
///
/// - **`Unknown` is not a miss.** It is "the ruleset has not answered yet", and
///   answering it as a miss would walk a tenancy toward eviction because an evaluator
///   was late. The caller hardens it first with [`crate::harden`] once
///   [`crate::UNKNOWN_GRACE`] periods have passed with no plan; a hardened `Unknown`
///   arrives here as `Unsupported`.
/// - **`EvictionPending` + `Unsupported` does nothing here.** The window's notice
///   decay needs an escalation id the pure library cannot mint, so E5b drives it
///   through [`window_due`] / [`window_elapsed`] after this call.
pub fn advance(t: &Tenancy, a: Assessment, now: i64) -> Result<Tenancy, HousingError> {
    if t.status.is_ended() {
        return Err(HousingError::AlreadyEnded(t.id));
    }
    if matches!(t.status, TenancyStatus::Pending) {
        return Err(HousingError::State(t.id));
    }
    match a {
        Assessment::Unknown => Ok(t.clone()),
        Assessment::Supported | Assessment::Strained => period_supported(t, now),
        Assessment::Unsupported => match t.status {
            TenancyStatus::Active => period_missed(t, now),
            TenancyStatus::Grace { since } => {
                if periods_since(since, now)? >= GRACE_PERIODS {
                    grace_elapsed(t, now)
                } else {
                    Ok(t.clone())
                }
            }
            TenancyStatus::Delinquent { since } => {
                if periods_since(since, now)? >= DELINQUENT_PERIODS {
                    delinquent_elapsed(t, now)
                } else {
                    Ok(t.clone())
                }
            }
            _ => Ok(t.clone()),
        },
    }
}

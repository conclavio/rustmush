//! The assessment tick (E5b, 08d §4's "assessment tick").
//!
//! ## A pull, not a subscription
//!
//! 08d §4 says housing "subscribes to the E4 cadence clock named by each tenancy".
//! There is no subscription seam on a cadence, and adding one would put a housing
//! callback inside the economy sweep — a module the economy must not know about,
//! called from the middle of a money tick. So this is the **pull** form of the same
//! rule, and it is strictly better behaved: housing keeps `last_assessed_position`
//! per tenancy on its own `Coarse` tick and assesses every tenancy whose cadence
//! `position` has moved past it. Nothing is lost if a fire is missed (the next one
//! sees the same gap), nothing double-fires (the position is the idempotency key),
//! and the economy needs no change at all.
//!
//! ## What one fire may do
//!
//! - at most [`crate::MAX_ASSESSMENTS_PER_FIRE`] tenancies, taken from a **rotation
//!   cursor** (E4b2's B4): the cap bounds *work*, and id order alone would starve the
//!   high-id tenancies of every fire, forever;
//! - **one assessed period per tenancy**, however far the clock jumped. A cadence that
//!   advanced forty periods over a weekend of downtime assesses **once** — the E4 rule
//!   that a backlog is never charged, in its housing form. The ladder is three
//!   assessed periods from the first unsupported one to the story window, and a
//!   restart must not spend all three at once while the player was asleep;
//! - **one private notice per transition, and none on a supported period**
//!   ([`crate::flow::announce`]).
//!
//! ## Abstract and exact
//!
//! An **abstract** tenancy is assessed from its residents' [`crate::SupportPlan`]s
//! through [`crate::assess_at`] — roster-checked and age-checked — then hardened, then
//! folded through [`crate::advance`]. An **exact** one is not: *whether* it is behind
//! comes from the E4 obligation, mirrored onto the ladder's first rung by
//! [`crate::econ`]; *how many assessed periods* it has been behind is a question only
//! this cadence can answer, so `Grace → Delinquent → EvictionPending` is driven here,
//! on the tenancy's own clock, exactly as it is for an abstract one.
//!
//! ## Exhaustive by construction
//!
//! Every match in this file is exhaustive and there is no `?`, no `unwrap`, no
//! `expect` and no panicking index: a tick handler runs in the world task, so a panic
//! here takes the world down (the discipline `mush-economy`'s sweep states and this
//! file inherits).

use mush_core::{Ctx, SceneId};

use crate::assess::{Assessment, PLAN_MAX_AGE_PERIODS, assess_at, harden};
use crate::ids::TenancyId;
use crate::module::{AssessState, HousingStore, MAX_ASSESSMENTS_PER_FIRE, put_assess, put_tenancy};
use crate::record::{Funding, Tenancy, TenancyStatus};
use crate::tenancy::{DELINQUENT_PERIODS, WindowAction, periods_since, window_due, window_elapsed};

/// One fire.
pub(crate) fn tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<SceneId>) {
    let (ids, cursor) = {
        let Some(store) = ctx.w.module_state::<HousingStore>() else { return };
        (crate::boot::tenancy_ids(store), store.cursor)
    };
    if ids.is_empty() {
        return;
    }
    // The rotation: start at the first id at or past the cursor, wrap once.
    let start = ids.iter().position(|id| id.0 >= cursor).unwrap_or(0);
    let order: Vec<TenancyId> =
        ids[start..].iter().chain(ids[..start].iter()).copied().collect();

    let mut worked = 0usize;
    let mut last = None;
    for id in order {
        if worked >= MAX_ASSESSMENTS_PER_FIRE {
            break;
        }
        match step(ctx, id) {
            Step::Skipped => {}
            Step::Worked => {
                worked += 1;
                last = Some(id);
            }
        }
    }
    // Resume **past** the last tenancy this fire reached, so the next one starts with
    // whoever was cut off rather than at the top of the list again.
    if let Some(last) = last {
        let Some(store) = ctx.w.module_state_mut::<HousingStore>() else { return };
        store.cursor = last.0.saturating_add(1);
        crate::module::persist_meta(ctx);
    }
}

/// What one tenancy cost this fire.
enum Step {
    /// Nothing to do: not live, no clock, or the clock has not moved.
    Skipped,
    /// One assessed period.
    Worked,
}

fn step(ctx: &mut Ctx<'_>, id: TenancyId) -> Step {
    let Some(t) = crate::flow::tenancy_of(ctx, id) else { return Step::Skipped };
    // A drafted lease whose contract has since activated: bind its rent obligation and
    // go live. The observer is the fast path; this is the one that survives a restart
    // between the acceptance and housing's own boot.
    if !t.status.is_live() {
        crate::econ::bind_if_ready(ctx, &t);
        return Step::Skipped;
    }
    // A tenancy whose cadence record is gone is **frozen**, not advanced: an
    // unresolvable clock never makes anything happen (the economy's own rule).
    let Some(now) = crate::module::cadence_now(ctx.w, t.cadence) else {
        return Step::Skipped;
    };
    let state = {
        let Some(store) = ctx.w.module_state::<HousingStore>() else { return Step::Skipped };
        store.assess_state(id)
    };
    // A tenancy created between two fires has never been assessed, and a zeroed
    // cursor on a clock at period 900 would read as "900 periods are owed". Seeding
    // is the first thing that happens to it and costs nothing else.
    if !state.seeded {
        put_assess(ctx, id, AssessState { position: now, period: 0, unknown_periods: 0, seeded: true });
        return Step::Skipped;
    }
    if now <= state.position {
        return Step::Skipped;
    }

    // **One period, however far the clock moved.**
    let period = state.period.saturating_add(1);
    let before = t.status.kind();
    let (next, unknown_periods) = match &t.funding {
        Funding::Abstract { plans } => {
            let tier = property_tier(ctx, &t);
            let raw = assess_at(tier, plans, &t.tenants, period, PLAN_MAX_AGE_PERIODS);
            let unknown = if raw == Assessment::Unknown {
                state.unknown_periods.saturating_add(1)
            } else {
                0
            };
            let verdict = harden(raw, unknown);
            match crate::advance(&t, verdict, now) {
                Ok(next) => (next, unknown),
                Err(error) => {
                    tracing::warn!(tenancy = id.0, %error, "housing: the ladder refused a step");
                    (t.clone(), unknown)
                }
            }
        }
        // Exact: the obligation drives Grace/Delinquent through the observer; the
        // only arrow left is the one it has no opinion about.
        Funding::Exact { obligation, .. } => (exact_step(ctx, &t, *obligation, now), 0),
    };
    put_assess(ctx, id, AssessState { position: now, period, unknown_periods, seeded: true });
    if next.status.kind() != before || next.revision != t.revision {
        crate::flow::commit(ctx, before, next);
    }
    // Whatever the ladder did, the window's own schedule is driven on the same
    // period: entering it opens the job, and every later period spends (or does not
    // spend) a notice.
    drive_window(ctx, id, now);
    Step::Worked
}

/// The ladder for an **exact** tenancy, on the tenancy's own cadence (08d §4).
///
/// The split with [`crate::econ`] is deliberate and is the same rule
/// [`mush_economy::EconomicDeadline`] enforces everywhere else: **two stamps are only
/// ever compared on the same clock.** The obligation says *whether* the tenant is
/// behind — it settles on its own schedule, which need not be the campaign clock this
/// tenancy is assessed on — and the observer mirrors that onto the ladder's first
/// rung the moment it happens. How many *assessed periods* they have been behind is a
/// question only this cadence can answer, so the two promotions live here.
///
/// A tenancy whose obligation has gone away is left exactly where it is: boot pages
/// about that (08d §4), and "the record we mirror is missing" is not evidence anybody
/// missed a payment.
fn exact_step(
    ctx: &Ctx<'_>,
    t: &Tenancy,
    obligation: mush_economy::ObligationId,
    now: i64,
) -> Tenancy {
    if crate::econ::is_pending_lease(&t.funding) {
        return t.clone();
    }
    let kind = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|s| s.snapshot().obligation(obligation).map(|o| o.status.kind()));
    // Only a **delinquent** obligation walks the ladder further down. Grace is where
    // the observer already put it, and everything else says nothing about the tenant.
    if !matches!(kind, Some(mush_economy::ObligationStatusKind::Delinquent)) {
        return t.clone();
    }
    let stepped = match t.status {
        TenancyStatus::Grace { since } => due(since, now, crate::tenancy::GRACE_PERIODS)
            .then(|| crate::grace_elapsed(t, now))
            .transpose(),
        TenancyStatus::Delinquent { since } => due(since, now, DELINQUENT_PERIODS)
            .then(|| crate::delinquent_elapsed(t, now))
            .transpose(),
        _ => Ok(None),
    };
    match stepped {
        Ok(Some(next)) => next,
        Ok(None) => t.clone(),
        Err(error) => {
            tracing::warn!(tenancy = t.id.0, %error, "housing: the exact ladder refused a step");
            t.clone()
        }
    }
}

/// Whether `wanted` periods have elapsed from `since` to `now`. An unreadable pair
/// (the `i64` extremes, which only a corrupt document holds) is **not due**: a clock
/// error must not push a home one rung closer to a story window.
fn due(since: i64, now: i64, wanted: i64) -> bool {
    matches!(periods_since(since, now), Ok(elapsed) if elapsed >= wanted)
}

/// The property's tier, or the lowest band when the property is somehow gone —
/// tier 0 is carried by every support level, so a missing property never walks a
/// tenancy toward eviction on a number nobody wrote.
fn property_tier(ctx: &Ctx<'_>, t: &Tenancy) -> crate::record::PropertyTier {
    ctx.w
        .module_state::<HousingStore>()
        .and_then(|s| s.property(t.property).map(|p| p.tier))
        .unwrap_or_default()
}

/// Open the story job on entry, then spend the window's decay schedule (§3.2).
///
/// [`window_due`] decides; this only executes. The `Escalate` arm mints the
/// [`mush_economy::EscalationId`] **first** and folds it in, because
/// [`window_elapsed`] refuses a bare `Escalate` (`NeedsEscalation`) rather than
/// silently skipping the page — a dropped escalation is how a home nobody looked at
/// becomes a home nobody ever looks at.
pub(crate) fn drive_window(ctx: &mut Ctx<'_>, id: TenancyId, now: i64) {
    let Some(t) = crate::flow::tenancy_of(ctx, id) else { return };
    if !matches!(t.status, TenancyStatus::EvictionPending { .. }) {
        return;
    }
    // Entry: the ST's ticket. Idempotent — `attach_window` refuses to replace a
    // different id and is a no-op for the same one.
    let t = match crate::flow::open_story_job(ctx, &t) {
        Some(job) => match crate::attach_window(&t, Some(job), None) {
            Ok(next) => {
                if next.revision != t.revision {
                    put_tenancy(ctx, next.clone());
                }
                next
            }
            Err(error) => {
                tracing::warn!(tenancy = id.0, %error, "housing: could not attach the story job");
                t
            }
        },
        None => t,
    };

    match window_due(&t, now) {
        WindowAction::Silent => {}
        WindowAction::Notify | WindowAction::NotifyEscalated => {
            match window_elapsed(&t, now, None) {
                Ok(next) => {
                    if next.revision != t.revision {
                        put_tenancy(ctx, next.clone());
                        renotify(ctx, &next);
                    }
                }
                Err(error) => {
                    tracing::warn!(tenancy = id.0, %error, "housing: a window re-notice was refused");
                }
            }
        }
        WindowAction::Escalate => {
            let escalation = crate::flow::page_window(ctx, &t);
            if escalation.is_none() {
                // B2's shape: the ladder could not mint a record. Say so loudly and
                // leave the window exactly where it is, so the next period asks again
                // rather than marking the escalation done.
                tracing::error!(
                    tenancy = id.0,
                    "housing: no escalation record could be opened for an eviction window"
                );
                return;
            }
            match window_elapsed(&t, now, escalation) {
                Ok(next) => {
                    put_tenancy(ctx, next.clone());
                    crate::flow::notify_tenants(
                        ctx,
                        &next,
                        &format!(
                            "HOME: tenancy #{} is still open and staff have now been paged about \
                             it. Nothing has changed about where you live.",
                            next.id
                        ),
                    );
                }
                Err(error) => {
                    tracing::error!(tenancy = id.0, %error, "housing: the escalation step was refused");
                }
            }
        }
    }
}

/// The decaying re-notice (§3.2): at most `MAX_WINDOW_NOTICES`, then silence until
/// the escalation, then the slow post-escalation rate.
fn renotify(ctx: &mut Ctx<'_>, t: &Tenancy) {
    let TenancyStatus::EvictionPending { window } = &t.status else { return };
    let n = window.notified;
    crate::flow::notify_tenants(
        ctx,
        t,
        &format!(
            "HOME: tenancy #{} is still waiting on a decision ({n} of {}). You still live there, \
             and nothing about your home has changed.",
            t.id,
            crate::MAX_WINDOW_NOTICES
        ),
    );
}

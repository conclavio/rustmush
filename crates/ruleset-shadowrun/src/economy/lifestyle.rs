//! Lifestyle upkeep — one fixed-amount obligation per profiled character on the
//! `sr.month` cadence (08f §3.2 "Life", D-E6-3, D-E6-23, M11, G12).
//!
//! ## The rule, exactly
//!
//! ```text
//! Paid       ──miss──▶ Arrears{1} = Grace          one private notice, NO downgrade
//! Arrears{1} ──miss──▶ Delinquent: one tier down; the record is REPLACED at the new
//!                      tier (create_obligation), unpaid_periods carried
//! Arrears{_} ──the next period pays / +lifestyle/pay: arrears, then reinstate──▶ Paid (silent)
//! any        ── ForgiveCause::{Downtime,TimeJump,Suspension} ──▶ unchanged, no downgrade
//! Grace | Delinquent ── +lifestyle/set ──▶ REFUSED (pay the arrears first)
//! ```
//!
//! Three facts about E4 shape it, and each is load-bearing:
//!
//! 1. **A downgrade is a replacement, not a rescale.** `Obligation.amount` is a fixed
//!    `Amount`, and `Delinquent` leaves the due pool with autopay stopped
//!    (`obligation.rs`, `sweep.rs`) — so the second miss cancels the record and
//!    creates a new one at the tier below, with `unpaid_periods` carried (M11).
//! 2. **The downgrade is driven by a *missed* period, never by the arrears counter's
//!    absolute value.** `plan_sweep` increments `missed_total` on a miss and
//!    `forgiven_total` on a forgiveness, so a downtime or clock-jump backlog — which
//!    is forgiven — moves nothing here. This module keeps the high-water mark it has
//!    already accounted for and acts on the **delta** alone.
//! 3. **One fire is one attempt.** The lifestyle schedule's `maximum_catch_up` is
//!    [`LIFESTYLE_CATCH_UP`] (1), so a ten-period backlog is one attempt and can
//!    never be the second miss; the `sr.month` clock's own `catch_up` (3) forgives
//!    on the clock first (D-E6-23, G12).
//!
//! Every money move here is E4's: [`mush_economy::create_obligation`] under the
//! actor's own debit authority, [`mush_economy::cancel_obligation`] and
//! [`mush_economy::reinstate_obligation`] under the same namespace lock, and one
//! ordinary attributed `post` for `+lifestyle/pay`. Nothing writes a sheet
//! (D-E6-16).

use mush_core::{Ctx, ObjId};
use mush_economy::{
    Amount, EconomicCause, EconomicClock, EconomicSchedule, EconomicVisibility, EconomyPlan,
    EconomyStore, GracePolicy, IdempotencyKey, Obligation, ObligationCause, ObligationSpec,
    ObligationStatus, ObligationStatusKind, RequestedLine,
};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::economy::boot::{LIFESTYLE_NAMESPACE, is_lifestyle_record, is_live, sink_account};
use crate::economy::config::{
    LIFESTYLE_CATCH_UP, LIFESTYLE_GRACE_PERIODS, LifestyleTier, SrEconomyConfig,
};
use crate::economy::record::SrProfile;
use crate::economy::store::{SrEconomyStore, put_profile};

/// How many arrears notices a character gets before the layer goes quiet (§9). The
/// ladder's own level-2 escalation is what speaks after that.
pub const MAX_ARREARS_NOTICES: u8 = 3;

/// Why a lifestyle change was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LifestyleError {
    /// The layer is off, or the clock/instrument/sink is missing.
    Unavailable(&'static str),
    /// The character has no profile or no account.
    NoProfile,
    /// `+lifestyle/set` while the record is in `Grace`/`Delinquent` (D-E6-23).
    InArrears,
    /// The economy refused the underlying seam.
    Economy(mush_economy::EconomyError),
}

impl std::fmt::Display for LifestyleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LifestyleError::Unavailable(why) => write!(f, "{why}"),
            LifestyleError::NoProfile => f.write_str("You have no nuyen account yet."),
            LifestyleError::InArrears => f.write_str(
                "Your lifestyle is in arrears. Pay them first (+lifestyle/pay), then set a tier.",
            ),
            LifestyleError::Economy(e) => write!(f, "{e}"),
        }
    }
}

/// The cadence's current period index, when the clock exists.
#[must_use]
pub fn cadence_now(ctx: &Ctx<'_>) -> Option<i64> {
    let cadence = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(SrEconomyStore::cadence)?;
    let position = ctx
        .w
        .module_state::<EconomyStore>()?
        .snapshot()
        .cadence(cadence)?
        .position;
    i64::try_from(position).ok()
}

/// This tier's per-period amount in minor units (scale 0, so major == minor for
/// `sr.nuyen`; the conversion is still done through the instrument's own scale).
#[must_use]
pub fn tier_amount(ctx: &Ctx<'_>, cfg: &SrEconomyConfig, tier: LifestyleTier) -> Option<Amount> {
    let major = cfg.cost_major(tier)?;
    let instrument = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.instrument.clone())?;
    let scale = ctx
        .w
        .module_state::<EconomyStore>()?
        .snapshot()
        .instrument(&instrument)?
        .scale;
    let factor = 10_i128.checked_pow(u32::from(scale))?;
    Amount::from_widened(i128::from(major).checked_mul(factor)?)
}

/// The live upkeep record for `who`, found the way DL-07 says: by the record's own
/// cause and creditor, with `SrProfile.upkeep` as a hint only.
#[must_use]
pub fn upkeep_of(ctx: &Ctx<'_>, who: ObjId) -> Option<Obligation> {
    let instrument = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.instrument.clone())?;
    let sink = sink_account(ctx, &instrument)?;
    let account = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.profile(who).and_then(|p| p.account))?;
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .obligations()
        .find(|o| is_lifestyle_record(o, sink) && is_live(o) && o.debit_account == account)
        .cloned()
}

/// Whether the live record says this character owes a period (the `+lifestyle/set`
/// gate — the record is authority, the profile counter is a cache).
#[must_use]
pub fn record_in_arrears(o: &Obligation) -> bool {
    matches!(
        o.status,
        ObligationStatus::Grace { .. } | ObligationStatus::Delinquent
    )
}

/// `+lifestyle/set <tier>` — **applies next period** (D-E6-3): a tier change
/// *replaces* the obligation, because `Obligation.amount` is fixed.
///
/// Refused in arrears (D-E6-23), so a tier change can never wipe a miss. A zero-cost
/// tier creates no obligation at all — which is what makes `street` a terminal floor
/// for the downgrade ladder.
///
/// # Errors
///
/// [`LifestyleError::Unavailable`] (the layer is off, or the clock, instrument or
/// sink is missing), [`LifestyleError::NoProfile`], [`LifestyleError::InArrears`],
/// [`LifestyleError::Economy`] (the seam refused: no debit authority, a closed
/// account, a frozen economy).
pub fn set_tier(
    ctx: &mut Ctx<'_>,
    cfg: &SrEconomyConfig,
    who: ObjId,
    tier: LifestyleTier,
) -> Result<Option<String>, LifestyleError> {
    if !cfg.lifestyle_on() {
        return Err(LifestyleError::Unavailable(
            "Lifestyle upkeep is not enabled on this server.",
        ));
    }
    let Some(mut profile) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.profile(who).cloned())
    else {
        return Err(LifestyleError::NoProfile);
    };
    if profile.account.is_none() {
        return Err(LifestyleError::NoProfile);
    }
    let live = upkeep_of(ctx, who);
    if profile.in_arrears() || live.as_ref().is_some_and(record_in_arrears) {
        return Err(LifestyleError::InArrears);
    }
    // Cancel first: one live upkeep record per character, always.
    if let Some(o) = &live {
        mush_economy::cancel_obligation(ctx, o.id, LIFESTYLE_NAMESPACE)
            .map_err(LifestyleError::Economy)?;
    }
    profile.lifestyle = tier;
    profile.upkeep = None;
    profile.revision = profile.revision.saturating_add(1);
    put_profile(ctx, profile.clone());
    match open_upkeep(ctx, cfg, &mut profile, tier) {
        Ok(Some(id)) => Ok(Some(format!(
            "Lifestyle set to {}; the next period bills obligation #{}.",
            tier.label(),
            id.0
        ))),
        Ok(None) => Ok(Some(format!(
            "Lifestyle set to {}; it costs nothing, so no upkeep is billed.",
            tier.label()
        ))),
        Err(e) => Err(e),
    }
}

/// Create the upkeep record for `tier`, or none when the tier is free.
///
/// The schedule is on the `sr.month` cadence, **one period at a time**, starting at
/// the *next* period — a tier change never re-bills the period the character has
/// already lived in (D-E6-3's "applies next period").
fn open_upkeep(
    ctx: &mut Ctx<'_>,
    cfg: &SrEconomyConfig,
    profile: &mut SrProfile,
    tier: LifestyleTier,
) -> Result<Option<mush_economy::ObligationId>, LifestyleError> {
    let Some(amount) = tier_amount(ctx, cfg, tier) else {
        return Ok(None);
    };
    let Some(account) = profile.account else {
        return Err(LifestyleError::NoProfile);
    };
    let Some(instrument) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.instrument.clone())
    else {
        return Err(LifestyleError::Unavailable(
            "The nuyen instrument is not registered.",
        ));
    };
    let Some(sink) = sink_account(ctx, &instrument) else {
        return Err(LifestyleError::Unavailable(
            "system:sr.lifestyle has no account.",
        ));
    };
    let Some(cadence) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(SrEconomyStore::cadence)
    else {
        return Err(LifestyleError::Unavailable(
            "The sr.month cadence clock does not exist.",
        ));
    };
    let Some(now) = cadence_now(ctx) else {
        return Err(LifestyleError::Unavailable(
            "The sr.month cadence clock does not exist.",
        ));
    };
    let spec = ObligationSpec {
        debit_account: account,
        credit_account: sink,
        amount,
        schedule: EconomicSchedule::new(
            EconomicClock::Cadence(cadence),
            now.saturating_add(1),
            Some(1),
            LIFESTYLE_CATCH_UP,
        ),
        grace: GracePolicy::new(LIFESTYLE_GRACE_PERIODS),
        notify_on_success: false,
        cause: ObligationCause::Ruleset {
            namespace: SmolStr::new_static(LIFESTYLE_NAMESPACE),
        },
    };
    let id = mush_economy::create_obligation(ctx, spec).map_err(LifestyleError::Economy)?;
    profile.upkeep = Some(id);
    profile.revision = profile.revision.saturating_add(1);
    put_profile(ctx, profile.clone());
    Ok(Some(id))
}

/// `+lifestyle/pay` — an attributed post of the arrears to `system:sr.lifestyle`
/// under the player's **own** authority, then a reinstate.
///
/// The economy does not collect arrears and will not: reinstating re-anchors the
/// schedule, so reinstating *is* forgiving them (`api.rs`'s M3 note). Enforcing "pay
/// first" is the ruleset's job, which is exactly this function — the post lands
/// before the reinstate, and a refused post reinstates nothing.
///
/// # Errors
///
/// [`LifestyleError::NoProfile`], [`LifestyleError::Unavailable`],
/// [`LifestyleError::Economy`] (no funds, no debit authority, a frozen economy).
pub fn pay_arrears(
    ctx: &mut Ctx<'_>,
    cfg: &SrEconomyConfig,
    who: ObjId,
) -> Result<String, LifestyleError> {
    if !cfg.lifestyle_on() {
        return Err(LifestyleError::Unavailable(
            "Lifestyle upkeep is not enabled on this server.",
        ));
    }
    let Some(mut profile) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.profile(who).cloned())
    else {
        return Err(LifestyleError::NoProfile);
    };
    let Some(account) = profile.account else {
        return Err(LifestyleError::NoProfile);
    };
    let owed = profile.arrears;
    if owed.minor() <= 0 {
        return Ok("You owe no lifestyle arrears.".to_string());
    }
    let Some(instrument) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.instrument.clone())
    else {
        return Err(LifestyleError::Unavailable(
            "The nuyen instrument is not registered.",
        ));
    };
    let Some(sink) = sink_account(ctx, &instrument) else {
        return Err(LifestyleError::Unavailable(
            "system:sr.lifestyle has no account.",
        ));
    };
    let Some(negative) = owed.checked_neg() else {
        return Err(LifestyleError::Unavailable(
            "That arrears amount does not negate.",
        ));
    };
    let mut plan = EconomyPlan::new(
        // Keyed on the ordinal the arrears are keyed on, so a double-submitted
        // `+lifestyle/pay` pays once (E5c T3: never wall time).
        IdempotencyKey::new(
            SmolStr::new_static("sr.lifestyle.pay"),
            SmolStr::new(format!("{}/{}", who.0, profile.unpaid_since)),
        ),
        PrincipalId::Object(who),
        EconomicCause::Ruleset {
            namespace: SmolStr::new_static("shadowrun"),
            correlation: SmolStr::new_static("lifestyle.arrears"),
        },
        vec![
            RequestedLine {
                account,
                delta: negative,
                memo: SmolStr::new_static("lifestyle arrears"),
            },
            RequestedLine {
                account: sink,
                delta: owed,
                memo: SmolStr::new_static("lifestyle arrears"),
            },
        ],
    );
    plan.description = "Lifestyle arrears.".to_string();
    plan.visibility = EconomicVisibility::Parties;
    mush_economy::post(ctx, &plan).map_err(LifestyleError::Economy)?;
    profile.arrears = Amount::ZERO;
    profile.unpaid_periods = 0;
    profile.revision = profile.revision.saturating_add(1);
    put_profile(ctx, profile.clone());
    let reinstated = match upkeep_of(ctx, who) {
        Some(o) => mush_economy::reinstate_obligation(ctx, o.id, LIFESTYLE_NAMESPACE)
            .map_err(LifestyleError::Economy)?,
        None => false,
    };
    Ok(if reinstated {
        "Arrears paid; your lifestyle is reinstated.".to_string()
    } else {
        "Arrears paid.".to_string()
    })
}

/// The observer's driver: an upkeep record changed status.
///
/// Runs from the board observer's `ObligationChanged` arm, exhaustive and panic-free.
/// The **delta** is what acts: a miss is `missed_total` above the high-water mark
/// this profile has already accounted for, so a forgiven backlog — which increments
/// `forgiven_total` and never `missed_total` — moves nothing.
pub fn on_obligation_changed(
    ctx: &mut Ctx<'_>,
    cfg: &SrEconomyConfig,
    id: mush_economy::ObligationId,
    kind: ObligationStatusKind,
) {
    if !cfg.lifestyle_on() {
        return;
    }
    let Some(instrument) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.instrument.clone())
    else {
        return;
    };
    let Some(sink) = sink_account(ctx, &instrument) else {
        return;
    };
    let Some(o) = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().obligation(id).cloned())
    else {
        return;
    };
    if !is_lifestyle_record(&o, sink) {
        return;
    }
    let Some(who) = ctx.w.module_state::<SrEconomyStore>().and_then(|s| {
        s.profiles()
            .find(|p| p.upkeep == Some(id) || p.account == Some(o.debit_account))
            .map(|p| p.obj)
    }) else {
        return;
    };
    let Some(profile) = ctx
        .w
        .module_state::<SrEconomyStore>()
        .and_then(|s| s.profile(who).cloned())
    else {
        return;
    };
    match kind {
        // A period settled (or the record resumed): silent, and the counters clear.
        ObligationStatusKind::Active | ObligationStatusKind::Satisfied => clear(ctx, profile),
        ObligationStatusKind::Grace => miss(ctx, cfg, profile, &o, false),
        ObligationStatusKind::Delinquent => miss(ctx, cfg, profile, &o, true),
        // Out of the due pool for a human decision, or ended by one. Neither is a
        // missed period, and neither downgrades anything.
        ObligationStatusKind::Suspended | ObligationStatusKind::Cancelled => {}
    }
}

/// A settled period: the run of misses is over, silently (§3.2 "Life").
fn clear(ctx: &mut Ctx<'_>, mut profile: SrProfile) {
    if profile.unpaid_periods == 0 && profile.arrears.minor() == 0 {
        return;
    }
    profile.unpaid_periods = 0;
    profile.arrears = Amount::ZERO;
    profile.revision = profile.revision.saturating_add(1);
    put_profile(ctx, profile);
}

/// One missed period. `downgrade` is set on the second — the record is `Delinquent`,
/// which is exactly when E4 stops autopaying it.
fn miss(
    ctx: &mut Ctx<'_>,
    cfg: &SrEconomyConfig,
    mut profile: SrProfile,
    o: &Obligation,
    downgrade: bool,
) {
    let seen = u32::from(profile.unpaid_periods);
    if o.missed_total <= seen {
        // Nothing new was missed — a re-emitted event, a suspension coming back, or a
        // forgiven backlog. A forgiven period is never a miss.
        return;
    }
    let who = profile.obj;
    let ordinal = cadence_now(ctx).unwrap_or(profile.unpaid_since);
    profile.unpaid_periods = u8::try_from(o.missed_total).unwrap_or(u8::MAX);
    profile.unpaid_since = ordinal;
    profile.arrears = profile
        .arrears
        .checked_add(o.amount)
        .unwrap_or(profile.arrears);
    profile.revision = profile.revision.saturating_add(1);
    put_profile(ctx, profile.clone());
    if profile.unpaid_periods <= MAX_ARREARS_NOTICES {
        let text = if downgrade {
            format!(
                "LIFESTYLE: you missed a second period. Your lifestyle drops to {}. \
                 (+lifestyle/pay clears the arrears.)",
                profile.lifestyle.downgraded().label()
            )
        } else {
            "LIFESTYLE: you missed a period. Nothing has changed yet — pay before the next one \
             (+lifestyle/pay)."
                .to_string()
        };
        mush_economy::deliver_notice(ctx, who, text);
    }
    if !downgrade {
        return;
    }
    apply_downgrade(ctx, cfg, profile, o);
}

/// **At most one tier per unpaid period** (answer 3, M11): the record is replaced at
/// the tier below, with `unpaid_periods` and `arrears` carried across.
fn apply_downgrade(
    ctx: &mut Ctx<'_>,
    cfg: &SrEconomyConfig,
    mut profile: SrProfile,
    o: &Obligation,
) {
    let next = profile.lifestyle.downgraded();
    if next == profile.lifestyle {
        // Already at the floor; there is nothing below `street` and it costs nothing.
        return;
    }
    if let Err(error) = mush_economy::cancel_obligation(ctx, o.id, LIFESTYLE_NAMESPACE) {
        tracing::error!(
            obligation = o.id.0, who = profile.obj.0, %error,
            "sr.economy: could not cancel a delinquent lifestyle record for its downgrade; the \
             tier is unchanged"
        );
        return;
    }
    profile.lifestyle = next;
    profile.upkeep = None;
    profile.revision = profile.revision.saturating_add(1);
    put_profile(ctx, profile.clone());
    match open_upkeep(ctx, cfg, &mut profile, next) {
        Ok(_) => tracing::info!(
            who = profile.obj.0,
            tier = next.label(),
            "sr.economy: a second missed period downgraded a lifestyle by one tier"
        ),
        Err(error) => tracing::warn!(
            who = profile.obj.0, tier = next.label(), %error,
            "sr.economy: the downgraded lifestyle record could not be opened; the character is \
             at the new tier with no upkeep (+lifestyle/set re-creates it)"
        ),
    }
}

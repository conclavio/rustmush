//! The support assessment (E5a, 08d §3.3) — **D-E5-13: best-of across residents.**
//!
//! Housing asks one question per cadence period: *does anybody living here still
//! carry this tier?* It asks it of the [`crate::SupportPlan`]s the ruleset submitted
//! and of nothing else — no sheet, no wallet, no dots (D-E5-5). The answer feeds
//! [`crate::advance`], which is the only thing that moves a tenancy's status.
//!
//! **Best-of, not worst-of, and not a sum.** Any one paying tenant covers a shared
//! lease, so a co-tenant whose fortunes collapse cannot drag a housemate's home into
//! eviction — and, symmetrically, two poor residents do not add up to one solvent one.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::ObjId;

use crate::record::{PropertyTier, SupportLevel, SupportPlan};

/// Periods an `Unknown` assessment is tolerated before it counts as a miss (§9).
pub const UNKNOWN_GRACE: u32 = 1;

/// Periods a [`SupportPlan`] stays current before [`assess_at`] stops counting it
/// (§9).
///
/// A plan is an assertion about *this* period, not a standing fact: the ruleset
/// restates it every period for a character it still evaluates. Counting one forever
/// meant an evaluator that stopped — a crashed ruleset, a character nobody re-runs,
/// a resident whose sheet went away — left the last thing it ever said propping up the
/// lease indefinitely, and the `Unknown`/[`UNKNOWN_GRACE`] machinery that exists for
/// exactly that case could never fire, because the plan map was never empty.
pub const PLAN_MAX_AGE_PERIODS: u64 = 2;

/// What the residents' asserted means say about this tier, this period.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Assessment {
    /// Somebody comfortably carries it.
    Supported,
    /// Somebody carries it, but only just — one band short. The tenancy does **not**
    /// slip: the rent is being paid, at a cost the ruleset turns into strain (§5.2).
    /// The ladder treats this exactly like `Supported`.
    Strained,
    /// Nobody carries it.
    Unsupported,
    /// Nobody has answered — no resident has a plan.
    ///
    /// Deliberately **not** a miss (§3.3): a late or crashed evaluator must not walk
    /// a home toward eviction. It becomes `Unsupported` only after [`UNKNOWN_GRACE`]
    /// periods, through [`harden`].
    #[default]
    Unknown,
}

impl Assessment {
    /// Whether this assessment covers the tier — `Supported` or `Strained`.
    #[must_use]
    pub fn covers(self) -> bool {
        matches!(self, Assessment::Supported | Assessment::Strained)
    }
    /// A short human word.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Assessment::Supported => "supported",
            Assessment::Strained => "strained",
            Assessment::Unsupported => "unsupported",
            Assessment::Unknown => "unknown",
        }
    }
}

/// Assess a tier against the residents' plans — the **best** asserted support wins
/// (D-E5-13).
///
/// The band rule, on two opaque `0..=5` scalars and nothing else:
///
/// | best level vs tier | answer |
/// |---|---|
/// | `level >= tier` | [`Assessment::Supported`] |
/// | `level + 1 == tier` | [`Assessment::Strained`] |
/// | `level + 2 <= tier` | [`Assessment::Unsupported`] |
/// | no plans at all | [`Assessment::Unknown`] |
///
/// An empty map is `Unknown`, never `Unsupported`: "nobody told us" and "nobody can
/// pay" are different facts, and only one of them should cost somebody their home.
///
/// **This is the ageless, roster-blind form**: it believes every plan it is handed,
/// however old, and whoever it names. That is the right shape for a caller that has
/// already filtered — a render of a set of plans, a preview, a test — and the wrong
/// one for the periodic assessment, which must not be carried by an evaluator that
/// stopped running or by a resident who no longer lives there. [`assess_at`] is that
/// form, and it is what E5b's cadence tick calls.
#[must_use]
pub fn assess(tier: PropertyTier, plans: &BTreeMap<ObjId, SupportPlan>) -> Assessment {
    let Some(best) = plans.values().map(|p| p.level.get()).max() else {
        return Assessment::Unknown;
    };
    band(tier, best)
}

/// [`assess`] with the two facts a stored plan map cannot vouch for on its own: **who
/// still lives here**, and **how recently each of them said so**.
///
/// A plan counts only when its resident is in `tenants` and it was asserted within
/// `max_age` periods of `current_period` ([`PLAN_MAX_AGE_PERIODS`]). Everything else is
/// treated as **absent**, not as a miss — so a tenancy whose every plan has gone stale
/// assesses [`Assessment::Unknown`], and [`harden`] / [`UNKNOWN_GRACE`] then decide
/// what a silent evaluator costs. That is the whole point of the split: "nobody has
/// answered lately" must walk the same tolerant path as "nobody has answered yet",
/// rather than being an instant miss *or* an eternal free pass.
///
/// The roster check is the library-side half of E5b's boot reconciliation. A plan for
/// somebody who is no longer a tenant can only exist if the roster was edited around
/// [`crate::Tenancy::remove_tenant`] (which drops the plan with the tenant), and a
/// dead resident's last assertion must not keep supporting a lease they are not on.
#[must_use]
pub fn assess_at(
    tier: PropertyTier,
    plans: &BTreeMap<ObjId, SupportPlan>,
    tenants: &BTreeSet<ObjId>,
    current_period: u64,
    max_age: u64,
) -> Assessment {
    let best = plans
        .iter()
        .filter(|(who, p)| current(who, p, tenants, current_period, max_age))
        .map(|(_, p)| p.level.get())
        .max();
    match best {
        Some(best) => band(tier, best),
        None => Assessment::Unknown,
    }
}

/// Whether one stored plan still speaks for this tenancy at `current_period`.
fn current(
    who: &ObjId,
    plan: &SupportPlan,
    tenants: &BTreeSet<ObjId>,
    current_period: u64,
    max_age: u64,
) -> bool {
    // Both the key and the plan's own `resident` must be a current tenant: the two
    // agree for everything `submit_plan` ever wrote, and a document where they do not
    // is a plan attributed to somebody who never asserted it.
    if !tenants.contains(who) || !tenants.contains(&plan.resident) || plan.resident != *who {
        return false;
    }
    // Saturating, not checked: a plan stamped in the *future* (a re-pointed clock, a
    // restored backup) reads as age 0 and counts, which is the same answer the ladder
    // gives a clock that moved backwards everywhere else in this crate.
    current_period.saturating_sub(plan.asserted_period) <= max_age
}

/// The band rule, on the two opaque `0..=5` scalars and nothing else.
fn band(tier: PropertyTier, best: u8) -> Assessment {
    let tier = tier.get();
    if best >= tier {
        return Assessment::Supported;
    }
    // `best < tier` here, so the subtraction cannot underflow; it is written checked
    // anyway because both sides come off persisted records.
    match tier.checked_sub(best) {
        Some(1) => Assessment::Strained,
        _ => Assessment::Unsupported,
    }
}

/// The best asserted level among the residents, for a render (`+home`, a story job).
/// `None` when nobody has a plan.
///
/// Rebuilt through [`SupportLevel::new`] rather than returned as stored: the newtype
/// is `#[serde(transparent)]`, so a hand-written document can hold a 255 inside one,
/// and its clamp only applies on `get()`. Handing that value back to a caller who
/// compares two `SupportLevel`s directly, or prints the field, would leak a band the
/// rest of the crate cannot express.
#[must_use]
pub fn best_level(plans: &BTreeMap<ObjId, SupportPlan>) -> Option<SupportLevel> {
    plans
        .values()
        .map(|p| p.level.get())
        .max()
        .map(SupportLevel::new)
}

/// Harden an [`Assessment::Unknown`] into a miss once it has persisted for
/// [`UNKNOWN_GRACE`] periods (§3.3).
///
/// `unknown_periods` is how many consecutive periods this tenancy has assessed
/// `Unknown` **including this one**, so [`UNKNOWN_GRACE`] of them are tolerated and
/// the next one is the miss. E5b keeps the counter: it is per-tenancy per-period
/// module state, not tenancy record state, so the pure library does not invent a
/// field for it. Every other assessment passes through untouched.
#[must_use]
pub fn harden(a: Assessment, unknown_periods: u32) -> Assessment {
    match a {
        Assessment::Unknown if unknown_periods > UNKNOWN_GRACE => Assessment::Unsupported,
        other => other,
    }
}

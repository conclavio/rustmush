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

use std::collections::BTreeMap;

use mush_core::ObjId;

use crate::record::{PropertyTier, SupportLevel, SupportPlan};

/// Periods an `Unknown` assessment is tolerated before it counts as a miss (§9).
pub const UNKNOWN_GRACE: u32 = 1;

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
#[must_use]
pub fn assess(tier: PropertyTier, plans: &BTreeMap<ObjId, SupportPlan>) -> Assessment {
    let Some(best) = plans.values().map(|p| p.level.get()).max() else {
        return Assessment::Unknown;
    };
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
#[must_use]
pub fn best_level(plans: &BTreeMap<ObjId, SupportPlan>) -> Option<SupportLevel> {
    plans.values().map(|p| p.level).max()
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

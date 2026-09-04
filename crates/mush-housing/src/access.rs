//! The access predicate (E5a, 08d §3.3) — **fail closed, always.**
//!
//! This is the whole of housing's enforcement model. E5b hangs it off
//! `HookPoint::MoveAttempt` and `HookPoint::CarryAttempt` and does nothing else:
//! there is no lock, no exit rewrite, no `home` routing (owner answer 10). So the one
//! property that matters here is that **every unmatched case is
//! [`Access::Refused`]** — a `match` with no catch-all "allow", no `unwrap_or(true)`,
//! and no path where an unreadable or contradictory record admits somebody.
//!
//! Four rules, from owner answers 8–9 and D-E5-4:
//!
//! - **Tenants and their guests always**, at every live status. A delinquent tenant
//!   still walks into their own home; pressure is strain, status and a story hook,
//!   never a locked door. Doc 08 §11.1's interim "service/access restrictions" step is
//!   deliberately dropped.
//! - **No implied landlord entry.** Legal authority, economic title and physical
//!   access stay separate: holding the title to a building admits nobody.
//!   [`Access::Inspection`] exists only under an **explicit** written term.
//! - **`WhileDelinquent` means while delinquent** — `Delinquent` or
//!   `EvictionPending`, and only while the stamp that says so is in the past.
//! - **Carry mirrors enter.** Somebody who may not walk in may not reach in:
//!   [`may_take`] is [`may_enter`] under a different name, so the two can never drift.
//!
//! The neutral refusal text is E5b's — the library returns the variant.

use mush_core::ObjId;
use mush_org::PrincipalId;

use crate::record::{AccessTerms, InspectionWhen, Tenancy};
use crate::tenancy::periods_since;

/// On what footing somebody reaches a room of a let property.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Access {
    /// A tenant, at any live status.
    Tenant,
    /// A tenant-managed guest, at any live status.
    Guest,
    /// The holder of an explicit inspection term, in a status that term admits.
    Inspection,
    /// Staff bypass — **the moved object being staff** (`check_move_attempt` sees the
    /// object that moves, not whoever issued the `@teleport`; 08d §1.1 scopes that).
    Staff,
    /// Everybody else, and every case this predicate does not recognise.
    #[default]
    Refused,
}

impl Access {
    /// Whether the hook should allow the move (or the carry).
    #[must_use]
    pub fn allowed(self) -> bool {
        !matches!(self, Access::Refused)
    }
}

/// May `who` enter (or take from) a room of the property under this tenancy?
///
/// `terms` is passed alongside the tenancy rather than read off it so a caller can
/// evaluate a **proposed** term without writing it down first (`@tenancy/create
/// inspect=…` previews its own policy). E5b passes `&t.terms`.
///
/// An inspection term naming an organization or a system principal cannot be
/// resolved from an [`ObjId`] alone, so it is **refused** here; E5b resolves org
/// membership and calls [`may_enter_with`] instead.
#[must_use]
pub fn may_enter(
    t: &Tenancy,
    terms: &AccessTerms,
    who: ObjId,
    is_staff: bool,
    now: i64,
) -> Access {
    let inspector = matches!(&terms.inspection, Some(term) if term.who == PrincipalId::Object(who));
    may_enter_with(t, terms, who, inspector, is_staff, now)
}

/// [`may_enter`] with the inspection-holder question already answered.
///
/// `inspector` is "does `who` hold the authority the term's [`PrincipalId`] names" —
/// an object principal is `who` themself, an organization principal is a member with
/// the landlord-scope capability, a system principal is whatever registered it. Only
/// E5b can answer that (it needs `mush_org::authorize_in_world`), so the pure library
/// takes the answer as a parameter and still owns *when* the term applies.
#[must_use]
pub fn may_enter_with(
    t: &Tenancy,
    terms: &AccessTerms,
    who: ObjId,
    inspector: bool,
    is_staff: bool,
    now: i64,
) -> Access {
    // A tenancy that is not live grants nothing to anybody. `Pending` has not been
    // agreed yet and `Ended` has been revoked — "regardless of status" (§3.3) is about
    // the failure ladder, never about a tenancy that is not in force.
    if t.status.is_live() {
        if t.is_tenant(who) {
            return Access::Tenant;
        }
        if t.is_guest(who) {
            return Access::Guest;
        }
        if inspector
            && let Some(term) = &terms.inspection
        {
            let admitted = match term.when {
                InspectionWhen::Always => true,
                // Only while the tenancy is actually behind — and only once the stamp
                // that says so is in the past. A cadence that moved backwards (an
                // epoch change, a re-pointed clock, a corrupt document) leaves a
                // `since` in the future, and reading that as "delinquent already"
                // would open a door on a clock error. Zero elapsed periods refuses.
                InspectionWhen::WhileDelinquent => {
                    t.status.is_behind()
                        && match t.status.since() {
                            Some(since) => now >= since && periods_since(since, now).is_ok(),
                            None => false,
                        }
                }
            };
            if admitted {
                return Access::Inspection;
            }
        }
    }
    // The staff bypass is the last word and applies at every status, so a staffer can
    // still reach a withdrawn or ended property to clean it up.
    if is_staff {
        return Access::Staff;
    }
    Access::Refused
}

/// May `who` take something out of a room of the property under this tenancy?
///
/// **Identical semantics to [`may_enter`], by construction** — it calls it. Doc 08's
/// rule is that an actor who may not enter may not take from inside, and the surest
/// way to keep two predicates in agreement is to have one.
#[must_use]
pub fn may_take(t: &Tenancy, terms: &AccessTerms, who: ObjId, is_staff: bool, now: i64) -> Access {
    may_enter(t, terms, who, is_staff, now)
}

/// [`may_take`] with the inspection-holder question already answered — the
/// [`may_enter_with`] counterpart.
#[must_use]
pub fn may_take_with(
    t: &Tenancy,
    terms: &AccessTerms,
    who: ObjId,
    inspector: bool,
    is_staff: bool,
    now: i64,
) -> Access {
    may_enter_with(t, terms, who, inspector, is_staff, now)
}

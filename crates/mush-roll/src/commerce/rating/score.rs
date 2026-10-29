//! Doc 17h §3 M8f (the lead's ruling P1): a revision's rating score, for
//! procurement's ranking.
//!
//! The certificate holds no win count (`ClassOutcome` is the ops budget, not
//! a result), so the score reads the result pages the certificate's
//! `RatingResultRef` names: over the revision's LOWEST passing class, the
//! share of its trial cases whose terminal is a `Win`, in per mille. A class
//! passes when no case of it ended `BudgetExceeded` (`ClassSummary::from_cases`'
//! own rule). A `Loss`, a `Draw`, a `Timeout` and a trial with no terminal are
//! not wins; probe, profile and kernel cases are not trials and count for
//! nothing. The score is computed at ranking time and never stored on the
//! Roll: the caller caches it (tacsim's procurement tick).
//!
//! A rating holds up to `MAX_PAGES` pages, more than one bounded owner read
//! carries, so the count is a [`RatingTally`] that folds pages in any order
//! and merges across reads; [`rating_score`] is the fold over a whole set.

use super::{CaseKind, CaseOutcome, PageContent, RatingClass, TrialOutcome};
use serde::Serialize;

/// One revision's score: `wins` of `cases` trial cases of `class`, its lowest
/// passing class, as `permille = wins × 1000 / cases`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct RatingScore {
    pub class: RatingClass,
    pub wins: u32,
    pub cases: u32,
    pub permille: u32,
}

/// One class's running count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
struct ClassTally {
    /// Any case of the class was read.
    seen: bool,
    /// Any case of the class ended `BudgetExceeded`: the class does not pass.
    faulted: bool,
    /// Trial cases.
    cases: u32,
    /// Trial cases whose terminal is a `Win`.
    wins: u32,
}
impl ClassTally {
    fn merge(&mut self, other: Self) -> Option<()> {
        self.seen |= other.seen;
        self.faulted |= other.faulted;
        self.cases = self.cases.checked_add(other.cases)?;
        self.wins = self.wins.checked_add(other.wins)?;
        Some(())
    }
}

/// The running count of a rating's result pages, per class (see the module
/// note). Pages may be added in any order and tallies merged; a page added
/// twice counts twice, so the caller adds each page once.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RatingTally {
    classes: [ClassTally; 3],
}
impl RatingTally {
    /// Count one page's rows. `None` on an arithmetic overflow (never for a
    /// valid rating, whose cases are bounded by `MAX_CASES`).
    #[must_use]
    pub fn add_page(mut self, page: &PageContent) -> Option<Self> {
        for row in page.rows() {
            let index = usize::from(row.key().class().ordinal()).checked_sub(1)?;
            let slot = self.classes.get_mut(index)?;
            slot.seen = true;
            if matches!(row.outcome(), CaseOutcome::BudgetExceeded {}) {
                slot.faulted = true;
            }
            if matches!(row.key().kind(), CaseKind::Trial { .. }) {
                slot.cases = slot.cases.checked_add(1)?;
                if row
                    .terminal()
                    .is_some_and(|end| end.outcome() == TrialOutcome::Win)
                {
                    slot.wins = slot.wins.checked_add(1)?;
                }
            }
        }
        Some(self)
    }
    /// The two counts together (two reads of disjoint page ranges).
    #[must_use]
    pub fn merge(mut self, other: Self) -> Option<Self> {
        for (mine, theirs) in self.classes.iter_mut().zip(other.classes) {
            mine.merge(theirs)?;
        }
        Some(self)
    }
    /// The score of everything counted: the lowest class read that passes;
    /// `None` when none passes or that class has no trial case.
    #[must_use]
    pub fn score(&self) -> Option<RatingScore> {
        let (index, tally) = self
            .classes
            .iter()
            .enumerate()
            .find(|(_, tally)| tally.seen && !tally.faulted)?;
        if tally.cases == 0 {
            return None;
        }
        let class = match index {
            0 => RatingClass::I,
            1 => RatingClass::II,
            _ => RatingClass::III,
        };
        Some(RatingScore {
            class,
            wins: tally.wins,
            cases: tally.cases,
            permille: tally.wins.checked_mul(1000)?.checked_div(tally.cases)?,
        })
    }
}

/// The score of the revision whose result pages are `pages` (see the module
/// note). `None` when no class passes, or when the lowest passing class has
/// no trial case.
#[must_use]
pub fn rating_score(pages: &[PageContent]) -> Option<RatingScore> {
    pages
        .iter()
        .try_fold(RatingTally::default(), RatingTally::add_page)?
        .score()
}

/// Procurement's rank key: `permille × 1000 / price_minor` (checked). A
/// price of zero or below is unrankable and answers `None`, never a division.
#[must_use]
pub fn score_per_credit(permille: u32, price_minor: i64) -> Option<u64> {
    let price = u64::try_from(price_minor).ok().filter(|p| *p > 0)?;
    u64::from(permille).checked_mul(1000)?.checked_div(price)
}

#[cfg(test)]
mod tests;

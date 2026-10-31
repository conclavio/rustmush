//! **The procurement choice, pure** (doc 17h §3 M8f P1, P4–P7): the month's spend from the
//! ledger rows, D-UM-54's cap, a full month, and the candidates' rank by score per credit.
//! The tick ([`super::tick`]) gathers the facts; everything it decides is decided here.
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use mush_core::CommitToken;
use mush_market::mind_commands::ListingForm;
use mush_roll::commerce::{CharterId, LicenceId};

use super::ledger::{Period, ProcurementRow, RowPhase};
use crate::launch::ProcurementRole;

/// Listings one month's scan reads and scores at most (P1's config-free bound; H4: the scores
/// are kept for the month, so the bound is on the scan).
pub const SCORE_CACHE_BOUND: usize = 256;

/// **The month's spend** (P7): the sum of `period`'s rows that count (not refunded). `None` on
/// an overflow, which the tick reads as "spent" (it buys nothing).
#[must_use]
pub fn spent(rows: &[ProcurementRow], period: Period) -> Option<i64> {
    rows.iter()
        .filter(|row| row.period == period && row.counts())
        .try_fold(0_i64, |sum, row| sum.checked_add(row.price_minor))
}

/// **A full month** (H6): `period` holds `limit` rows or more, so one read no longer sees them
/// all; the month closes rather than mis-count its spend or reuse an ordinal.
#[must_use]
pub fn ledger_full(rows: &[ProcurementRow], period: Period, limit: usize) -> bool {
    rows.iter().filter(|row| row.period == period).count() >= limit
}

/// **The diversity cap** (P6, D-UM-54): the most licences of one charter an organisation of
/// `hulls` titled hulls may hold, `floor(cap_permille × hulls / 1000)` (ten hulls at 400‰ →
/// four).
#[must_use]
pub fn cap_for(cap_permille: u16, hulls: u32) -> u32 {
    let seats = u64::from(cap_permille).saturating_mul(u64::from(hulls)) / 1000;
    u32::try_from(seats).unwrap_or(u32::MAX)
}

/// **The licences held per charter** (P6): the Roll's current licences, plus the ledger's
/// `Funded` rows (paid, not yet granted, so the Roll does not list them yet), each licence
/// once. A granted row is the Roll's to count: once its term lapses or is revoked it no longer
/// holds a seat.
#[must_use]
pub fn held_per_charter(
    holdings: &[(LicenceId, CharterId)],
    rows: &[ProcurementRow],
) -> BTreeMap<CharterId, u32> {
    let mut seen: BTreeSet<LicenceId> = BTreeSet::new();
    let mut held: BTreeMap<CharterId, u32> = BTreeMap::new();
    let funded = rows
        .iter()
        .filter(|row| row.phase == RowPhase::Funded)
        .map(|row| (row.licence, row.charter));
    for (licence, charter) in holdings.iter().copied().chain(funded) {
        if seen.insert(licence) {
            let count = held.entry(charter).or_insert(0);
            *count = count.saturating_add(1);
        }
    }
    held
}

/// Whether a policy's `roles` admit a listing's form (P2).
#[must_use]
pub fn admits(roles: &[ProcurementRole], form: ListingForm) -> bool {
    let role = match form {
        ListingForm::Sealed => ProcurementRole::Sealed,
        ListingForm::Open => ProcurementRole::Open,
        ListingForm::OpenResale => ProcurementRole::OpenResale,
    };
    roles.contains(&role)
}

/// One listing the tick may buy, with its rating's score.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub listing: CommitToken,
    pub charter: CharterId,
    pub price_minor: i64,
    pub permille: u32,
}
impl Candidate {
    /// **Its rank against `other`**, best first: score per credit, then the score, then the
    /// cheaper, then the token (a total order, so a choice never depends on read order).
    ///
    /// Score per credit is compared exactly, `self.permille × other.price` against
    /// `other.permille × self.price` in `u128`, never as a truncated quotient: under a 2,000cr
    /// budget `permille × 1000 / price_minor` is a single digit, so lines that really differ
    /// per credit tied (the M8-1 review finding; doc 17h §7 M8f amendment). Both prices are
    /// above zero here ([`choose`] admits no other).
    fn rank(&self, other: &Self) -> Ordering {
        let per_credit =
            |c: &Self, d: &Self| u128::from(c.permille) * u128::from(d.price_minor.unsigned_abs());
        per_credit(other, self)
            .cmp(&per_credit(self, other))
            .then_with(|| other.permille.cmp(&self.permille))
            .then_with(|| self.price_minor.cmp(&other.price_minor))
            .then_with(|| self.listing.cmp(&other.listing))
    }
}

/// **The choice** (P4–P6): among `candidates`, those priced within `remaining` (and above
/// zero: an unrankable price is never bought, P1), not `skipped`, whose charter is below `cap`
/// in `held`, the best by score per credit.
#[must_use]
pub fn choose<'a>(
    candidates: &'a [Candidate],
    remaining: i64,
    held: &BTreeMap<CharterId, u32>,
    cap: u32,
    skipped: &BTreeSet<CommitToken>,
) -> Option<&'a Candidate> {
    candidates
        .iter()
        .filter(|c| {
            c.price_minor > 0
                && c.price_minor <= remaining
                && !skipped.contains(&c.listing)
                && held.get(&c.charter).copied().unwrap_or(0) < cap
        })
        .min_by(|a, b| a.rank(b))
}

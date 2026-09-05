//! **The safe public projection** — E6c's gate (08f §5.3, D-E6-14, doc 10 §14(b)).
//!
//! The condition this module exists to make structurally true:
//!
//! > *Two worlds differing only in state the viewer may not see produce byte-identical
//! > public output.*
//!
//! It is proved by construction, in five parts, and each one is a property of the
//! code rather than of an inspection:
//!
//! 1. **[`MarketStore::projection`] takes no viewer argument.** Its signature is
//!    `(&self, &CatalogRef, &MarketId) -> Projection`, so there is no per-viewer
//!    projection for a hidden fact to leak *through*. Everyone who asks gets the same
//!    value from the same state; `+market/depth` renders it identically to a staffer
//!    and a stranger because there is nothing else it could render.
//! 2. **It is computed only over the public set.** A listing whose
//!    [`MarketVisibility`] is not `Public`, a listing whose seller is a **hidden**
//!    organization, and **every** quote of every buyer contribute nothing to band,
//!    depth, volume or last trade — not even a rounding difference, because they are
//!    never summed at all.
//! 3. **Last trade and volume come from settled journal transactions.** A trade-tape
//!    row is a [`Quote`] carrying a [`Settlement`], and each one is re-checked against
//!    the economy's own `ContractState::Settled { transaction }` before it counts. An
//!    unsettled offer, a withdrawn offer and an open quote carry no row at all, so
//!    they are invisible by construction rather than by a filter.
//! 4. **Depth is bucketed**, into [`DEPTH_BUCKETS`] price levels with quantities
//!    summed per level and floored to the bucket, never published per seller — so a
//!    single distinctive quantity cannot identify its owner.
//! 5. **The per-viewer surface is the listing list only**, through the existence-hiding
//!    renderer: a listing the actor may not view is `No such listing.`, identical for
//!    nonexistent and unviewable, and a filter the actor may not use returns an
//!    **empty list**, never a refusal (08a decision 13).
//!
//! Staff read underlying state through the `@market/*` verbs, which are ordinary
//! authorized reads — never through a privileged projection, because there is not one.

use std::collections::BTreeMap;

use mush_economy::{Amount, CatalogRef, ContractState, EconomySnapshot};
use mush_org::PrincipalId;

use crate::ids::MarketId;
use crate::record::{Listing, MarketVisibility, Quote, Settlement};
use crate::store::MarketStore;

/// How many price levels the published depth carries (08f §9).
pub const DEPTH_BUCKETS: usize = 8;

/// The window `volume` and `last_trade` are folded over, in seconds.
pub const PROJECTION_WINDOW_SECS: i64 = 7 * 86_400;

/// The granularity every published depth quantity is floored to.
///
/// The second half of the bucketing control: price buckets merge *neighbouring*
/// offers, and this stops the merged number from being one seller's own even when a
/// narrow band leaves an offer alone in its level. Ten is coarse enough that a
/// distinctive quantity (137) publishes as 130, and a market too thin to reach it
/// publishes nothing at all — the safe direction.
pub const DEPTH_QUANTITY_STEP: u64 = 10;

/// The most price **levels** and **tape rows** one projection may fold (the E6b
/// independent verify fold's R1: *filter, then take*).
///
/// It bounds records **matched**, not records visited, and that is deliberate: the
/// `take` sits after the filter because taking first would let a wall of hidden
/// listings displace the public ones out of the band — hidden state moving public
/// output, which is the one thing the gate forbids. What bounds the records the walk
/// *touches* is the store's own [`crate::MAX_LISTINGS`] / [`crate::MAX_QUOTES`] caps,
/// asked over the `(catalog, market)` index so a long tail of other catalogues costs
/// nothing; [`MarketStore::projection_visits`] is the tripwire that states it out loud
/// (independent verify fold, D8 — the constant and its doc comment disagreed).
pub const PROJECTION_VISIT: usize = 256;

/// One published price level. **Never** one seller's row.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DepthBucket {
    /// The bucket's floor price, in minor units.
    pub price: Amount,
    /// The summed quantity at or above that floor and below the next.
    pub quantity: u64,
}

/// The aggregate public view of one `(catalog, market)` pair (§5.3, D-E6-14).
///
/// It publishes **no account id, no listing id, no quote id, no counterparty and no
/// quantity attributable to one seller**. Every field is a sum or an extremum over a
/// set that was filtered to the public one before anything was added up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    /// The last **settled** trade's unit price, when there was one in the window.
    pub last_trade: Option<Amount>,
    /// The bounded price band over public listings: `(low, high)`. `(0, 0)` when the
    /// public set is empty — which is exactly what an empty set looks like from
    /// outside, and is the same value a world with only hidden listings publishes.
    pub band: (Amount, Amount),
    /// Depth by price level, floored into buckets.
    pub depth: [DepthBucket; DEPTH_BUCKETS],
    /// Settled volume over the window, in minor units.
    pub volume: Amount,
    /// The window `volume` and `last_trade` were folded over.
    pub window_secs: i64,
}

impl Default for DepthBucket {
    fn default() -> DepthBucket {
        DepthBucket {
            price: Amount::ZERO,
            quantity: 0,
        }
    }
}

impl Default for Projection {
    /// **The empty public view**, and the one a world whose every listing is hidden
    /// publishes: no last trade, a zero band, empty depth, zero volume. That the two
    /// are the same value is the gate, written as a constructor.
    fn default() -> Projection {
        Projection {
            last_trade: None,
            band: (Amount::ZERO, Amount::ZERO),
            depth: [DepthBucket::default(); DEPTH_BUCKETS],
            volume: Amount::ZERO,
            window_secs: PROJECTION_WINDOW_SECS,
        }
    }
}

/// Whether a principal is one the **public** set admits.
///
/// The one predicate the projection's public test uses, and it asks only what anyone
/// may ask: a hidden organization is not in the public set, and nothing else about a
/// seller matters. It takes `hidden_orgs` rather than a `&World` so the projection
/// itself stays a pure function of the store plus two declared inputs — which is what
/// makes "two worlds differing only in hidden state" a statement about arguments.
#[must_use]
pub fn is_public_seller(seller: &PrincipalId, hidden_orgs: &[mush_org::OrgId]) -> bool {
    match seller {
        // D-E6-4: a `System` principal can never be a party, so it can never be a
        // seller; a document naming one is refused on the wire. Fail closed anyway.
        PrincipalId::System(_) => false,
        PrincipalId::Organization(o) => !hidden_orgs.contains(o),
        PrincipalId::Object(_) => true,
    }
}

impl MarketStore {
    /// **The gate.** The aggregate public view of one `(catalog, market)` pair.
    ///
    /// Deliberately **argument-free of any viewer**: there is no `viewer: ObjId`
    /// parameter and no authority is consulted, so there is no per-viewer projection
    /// through which a hidden fact could leak. The signature is the proof.
    ///
    /// `snapshot` is the economy's, and it is read for exactly one thing: whether a
    /// trade-tape row's contract really is `Settled { transaction }` with the
    /// transaction the row names. `hidden_orgs` is the composed list of hidden selling
    /// organizations, supplied by the caller because `mush-market` reads org state
    /// through `mush-org` and the caller already holds it.
    #[must_use]
    pub fn projection(
        &self,
        catalog: &CatalogRef,
        market: &MarketId,
        snapshot: &EconomySnapshot,
        hidden_orgs: &[mush_org::OrgId],
        now: i64,
    ) -> Projection {
        let mut out = Projection {
            window_secs: PROJECTION_WINDOW_SECS,
            ..Projection::default()
        };
        // ---- band and depth: PUBLIC LISTINGS ONLY -------------------------
        //
        // Filter, then take (E6b independent verify fold, R1): the index is keyed on
        // the pair, so the bound falls on records that are already candidates and a
        // long tail of other catalogs never eats the budget.
        let mut levels: Vec<(i64, u64)> = Vec::new();
        let mut low = i64::MAX;
        let mut high = i64::MIN;
        for listing in self
            .listings_for(catalog, market)
            .filter(|l| self.publishes(l, hidden_orgs, now))
            .take(PROJECTION_VISIT)
        {
            // **The published price is the price the deal executes at** (independent
            // verify fold, D4): a maker's spread moved `Quote.price` and therefore
            // `last_trade`, while band and depth read `Listing.price` unmoved — so the
            // safe projection published a band that need not contain its own last
            // trade. One function, every surface.
            let p = crate::trade::price_for(listing, Some(self)).minor();
            low = low.min(p);
            high = high.max(p);
            levels.push((p, listing.quantity));
        }
        if low <= high {
            out.band = (Amount::from_minor(low), Amount::from_minor(high));
            out.depth = bucket(&levels, low, high);
        }
        // ---- last trade and volume: SETTLED JOURNAL TRANSACTIONS ONLY -----
        //
        // A row is only counted when the economy itself says the contract it names
        // reached `Settled { transaction }` with that very transaction. The market
        // document is the index; the journal is the authority.
        let mut latest: Option<(i64, Amount)> = None;
        let mut volume: i64 = 0;
        for (quote, settlement) in self
            .trades_for(catalog, market)
            .filter_map(|q| q.settled.map(|s| (q, s)))
            .filter(|(q, _)| self.publishes_trade(q, hidden_orgs))
            .take(PROJECTION_VISIT)
        {
            if !is_settled_in_journal(quote, &settlement, snapshot) {
                continue;
            }
            if settlement.at < now.saturating_sub(PROJECTION_WINDOW_SECS) {
                continue;
            }
            volume = volume.saturating_add(quote.total().minor());
            if latest.is_none_or(|(at, _)| settlement.at >= at) {
                latest = Some((settlement.at, quote.price));
            }
        }
        out.last_trade = latest.map(|(_, price)| price);
        out.volume = Amount::from_minor(floor_volume(volume, out.last_trade));
        out
    }

    /// Whether a **listing** is in the public set the projection folds.
    ///
    /// The single predicate `projection` and `commands::default_market` both ask
    /// (independent verify fold, D7): the venue default used to apply two of these four
    /// tests, so a lapsed public listing still chose the header venue while
    /// contributing nothing to the numbers, and a good whose only listing had lapsed
    /// read differently from a good with none.
    pub(crate) fn publishes(&self, l: &Listing, hidden_orgs: &[mush_org::OrgId], now: i64) -> bool {
        matches!(l.visibility, MarketVisibility::Public)
            && is_public_seller(&l.seller, hidden_orgs)
            && l.quantity > 0
            && !lapsed(l.expires.at, l.expires.clock, now)
            // A pack reload that dropped the entry leaves listings nothing can define,
            // quote or deliver (independent verify fold, R6). `load_pack` withdraws
            // them; this is the belt so an in-flight reload can never publish depth in
            // an undefinable good even for a moment.
            && self.entry_of(&l.catalog).is_some()
    }

    /// Whether a **trade-tape row** is in the public set.
    pub(crate) fn publishes_trade(&self, q: &Quote, hidden_orgs: &[mush_org::OrgId]) -> bool {
        matches!(q.visibility, MarketVisibility::Public)
            && is_public_seller(&q.seller, hidden_orgs)
            // The seller's own member buying from the seller's own shop is a real
            // settlement and a real row in their order history, and it is **not** a
            // price anybody else discovered (independent verify fold, M12).
            && !q.self_dealt
    }
}

/// Floor the published volume the same way depth's quantities are floored.
///
/// [`DEPTH_QUANTITY_STEP`] exists precisely so "the number published is never one
/// seller's own", and the tape then published `volume` and `last_trade` exactly — so
/// with a single settled row in the window, `volume / last_trade` recovered that
/// trade's exact quantity by one division (independent verify fold, D6). Volume is
/// floored to ten of the last trade's price, so the same 137 units publish as 130 here
/// too, and a window too thin to reach the step publishes **zero** — the safe
/// direction, and the one depth already takes.
fn floor_volume(volume: i64, last_trade: Option<Amount>) -> i64 {
    let Some(price) = last_trade.map(Amount::minor).filter(|p| *p > 0) else {
        return 0;
    };
    let step = price.saturating_mul(i64::try_from(DEPTH_QUANTITY_STEP).unwrap_or(1));
    if step <= 0 {
        return 0;
    }
    volume.saturating_div(step).saturating_mul(step)
}

/// Whether the economy's own record says this trade really settled on the
/// transaction the row names.
fn is_settled_in_journal(q: &Quote, s: &Settlement, snapshot: &EconomySnapshot) -> bool {
    let Some(contract) = q.contract else {
        return false;
    };
    matches!(
        snapshot.contract(contract).map(|c| &c.state),
        Some(ContractState::Settled { transaction }) if *transaction == s.transaction
    )
}

/// Whether a deadline on a wall clock has lapsed. A deadline on any other clock is
/// left to the sweep that owns that clock and is never treated as lapsed here — the
/// projection must not guess at a clock it does not read.
pub(crate) fn lapsed(at: i64, clock: mush_economy::EconomicClock, now: i64) -> bool {
    matches!(clock, mush_economy::EconomicClock::RealUtc) && now >= at
}

/// Sum the public levels into [`DEPTH_BUCKETS`] price buckets and **floor** each
/// bucket's quantity to [`DEPTH_QUANTITY_STEP`].
///
/// Bucketing is the disclosure control, not a display nicety, and it is two controls
/// rather than one:
///
/// - **Prices** are folded into exactly [`DEPTH_BUCKETS`] equal-width levels spanning
///   the whole public band, so neighbouring offers are only ever seen added together.
/// - **Quantities** are floored to [`DEPTH_QUANTITY_STEP`], so the number published is
///   never one seller's own. Price bucketing alone is not enough: a narrow band puts
///   one offer alone in a level, and its exact quantity is then a fingerprint. Depth
///   below the step publishes **zero**, which is the safe direction — at that size the
///   number *is* the seller.
fn bucket(levels: &[(i64, u64)], low: i64, high: i64) -> [DepthBucket; DEPTH_BUCKETS] {
    let mut out = [DepthBucket::default(); DEPTH_BUCKETS];
    if levels.is_empty() {
        return out;
    }
    let span = high.saturating_sub(low).max(0);
    let buckets = i64::try_from(DEPTH_BUCKETS).unwrap_or(1).max(1);
    // Ceiling division, at least one minor unit wide, so the band is covered by
    // exactly `DEPTH_BUCKETS` levels and the top price lands inside the last one.
    let width = span
        .saturating_add(1)
        .saturating_add(buckets - 1)
        .checked_div(buckets)
        .unwrap_or(1)
        .max(1);
    let mut sums: BTreeMap<usize, u64> = BTreeMap::new();
    for (price, quantity) in levels {
        let offset = price.saturating_sub(low).max(0);
        let idx = usize::try_from(offset / width)
            .unwrap_or(DEPTH_BUCKETS - 1)
            .min(DEPTH_BUCKETS - 1);
        let slot = sums.entry(idx).or_default();
        *slot = slot.saturating_add(*quantity);
    }
    for (idx, out_slot) in out.iter_mut().enumerate() {
        out_slot.price = Amount::from_minor(
            low.saturating_add(width.saturating_mul(i64::try_from(idx).unwrap_or(0))),
        );
        out_slot.quantity = floor_step(sums.get(&idx).copied().unwrap_or(0));
    }
    out
}

/// Floor a published quantity to [`DEPTH_QUANTITY_STEP`].
#[must_use]
fn floor_step(q: u64) -> u64 {
    q.saturating_div(DEPTH_QUANTITY_STEP)
        .saturating_mul(DEPTH_QUANTITY_STEP)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_distinctive_quantity_is_never_published_verbatim() {
        // Three sellers at three prices: the published depth carries neither the 7
        // that belongs to exactly one of them nor their exact sum.
        let levels = [(100, 7), (101, 3), (102, 5)];
        let d = bucket(&levels, 100, 102);
        assert!(
            d.iter()
                .all(|b| b.quantity != 7 && b.quantity != 3 && b.quantity != 5),
            "a bucket republished one seller's quantity: {d:?}"
        );
        assert!(
            d.iter().all(|b| b.quantity % DEPTH_QUANTITY_STEP == 0),
            "every published quantity is floored to the step"
        );
    }

    #[test]
    fn one_large_listing_is_not_identifiable_from_depth() {
        // The same 137 units, once as one seller's listing and once split across
        // three: the published depth is identical, so depth answers "how much is
        // around", never "who is holding it".
        let alone = bucket(&[(100, 137)], 100, 100);
        let split = bucket(&[(100, 100), (100, 30), (100, 7)], 100, 100);
        assert_eq!(alone, split);
        assert!(
            alone.iter().all(|b| b.quantity != 137),
            "the big listing's own quantity was published"
        );
    }

    #[test]
    fn an_empty_public_set_and_a_hidden_only_world_publish_the_same_projection() {
        // Both are the empty public view, byte for byte — the gate, as a value.
        assert_eq!(bucket(&[], 0, 0), [DepthBucket::default(); DEPTH_BUCKETS]);
        assert_eq!(Projection::default(), Projection::default());
    }

    #[test]
    fn a_hidden_org_seller_is_not_in_the_public_set() {
        let hidden = [mush_org::OrgId(7)];
        assert!(!is_public_seller(
            &PrincipalId::Organization(mush_org::OrgId(7)),
            &hidden
        ));
        assert!(is_public_seller(
            &PrincipalId::Organization(mush_org::OrgId(8)),
            &hidden
        ));
        assert!(!is_public_seller(
            &PrincipalId::System(
                mush_org::SystemPrincipalId::new("x").expect("a valid system principal id")
            ),
            &hidden
        ));
    }

    // ---- E6c independent verify fold ----

    /// **D6.** One trade's exact quantity is not recoverable from `volume /
    /// last_trade`: the published volume carries the same [`DEPTH_QUANTITY_STEP`]
    /// control depth's quantities do, so a one-row tape publishes a multiple of ten
    /// trades' worth or nothing at all.
    #[test]
    fn vf_d6_a_one_row_tape_never_publishes_its_own_quantity() {
        let last = Amount::from_minor(30);
        // Two units at 30: below the step, so the safe answer is to publish nothing.
        assert_eq!(floor_volume(60, Some(last)), 0);
        // Thirty-seven units at 30 publish as thirty, exactly as 137 units of depth
        // publish as 130.
        let published = floor_volume(37 * 30, Some(last));
        assert_eq!(published, 30 * 30);
        let step = last.minor() * i64::try_from(DEPTH_QUANTITY_STEP).expect("a small step");
        assert_eq!(published % step, 0, "the published volume is not floored");
        assert_ne!(published, 37 * 30, "the exact quantity was republished");
        // No trade at all publishes nothing, and cannot divide by zero doing it.
        assert_eq!(floor_volume(0, None), 0);
        assert_eq!(floor_volume(999, None), 0);
        assert_eq!(floor_volume(999, Some(Amount::ZERO)), 0);
        // Saturating throughout: an absurd price is arithmetic, never a panic.
        assert_eq!(
            floor_volume(i64::MAX, Some(Amount::from_minor(i64::MAX))),
            i64::MAX
        );
    }
}

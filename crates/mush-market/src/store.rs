//! The market's module state and its five document families (namespace `market`).
//!
//! Built on `ruleset-shadowrun`'s `economy::store` pattern exactly, because that
//! pattern is what the E-series folds have already hardened:
//!
//! - **the RAM slot is the single runtime truth**, and every index on it is a
//!   *re-derived projection* — never authority, and never maintained anywhere but the
//!   one function that rebuilds it;
//! - **`put_*` are the single write paths**, so RAM, the indexes and the queued
//!   document always move together;
//! - **the allocator floor comes from every document's parsed key**, loaded or not
//!   (E6b verify fold R8): a key that parses is a claim on an id whether or not its
//!   body does, so a corrupt document plus a lost allocator can never re-mint a live
//!   id and write over the evidence;
//! - **the `live` set drives the caps and the pages**, and a closed record is `Del`'d
//!   so a list/withdraw loop mints no permanent documents;
//! - **[`load`] is total.** Every document it cannot use is skipped **loudly** and the
//!   world boots. The direction of each loss is stated in [`crate::record`]'s header
//!   and is deliberate in every case: the money is the economy's, and it survives.

use std::collections::{BTreeMap, BTreeSet};

use mush_core::{Ctx, ModuleOp, ModuleWrite, ObjId, Outbox};
use mush_economy::{CatalogRef, ContractId};
use mush_org::{OrgId, PrincipalId};

use smol_str::SmolStr;

use crate::ids::{ListingId, MarketId, MarketSourceId, QuoteId, VendorId};
use crate::record::{
    CatalogEntry, Listing, LotMatter, MAX_CATALOG_ENTRIES, MAX_LISTINGS, MAX_LISTINGS_PER_VENDOR,
    MAX_QUOTES, MAX_SOURCES, MAX_TRADE_ROWS, MAX_VENDORS, MarketSource, Quote, Representation,
    Supply, Vendor,
};

/// The module namespace.
pub const NS: &str = "market";

/// This crate's mechanics owner id (E7r2 P1).
///
/// It exists because the lot lift registers this crate's **first** authoritative action
/// and its first `PrincipalId::Scoped` holder key, both of which are stamped with the
/// module's own owner. `mush_inventory::owner` and `mush_industry::record::owner` are
/// the shape.
///
/// # Panics
/// Never: `"market"` is a canonical id by construction, asserted by a test.
#[must_use]
pub fn owner() -> mush_core::OwnerId {
    mush_core::OwnerId::new(NS).expect("static market owner id is canonical")
}
/// The allocator document.
pub const META_KEY: &str = "market:meta";

/// The most **settled** trade rows the tape keeps per `(catalog, market)` pair.
///
/// The tape is the projection's only input for last trade and volume, and it is a
/// window rather than a history: 32 rows per pair bounds the store against a busy
/// market without bounding the *world's* history, which is the journal's and is
/// complete. Beyond it the oldest row is deleted, exactly as a closed listing is.
pub const MAX_TRADES_PER_PAIR: usize = 32;

/// The most listing or quote records one read may **visit** (D-LBK-5): a bounded read
/// bounds the records it *touches*, never the ones it *matches*.
pub const MARKET_PAGE_VISIT: usize = 256;

/// The most rows one market page renders (08i §9 answer 11: at most 256 opaque
/// choices per offer, applied to every page and fan-out in this cut).
pub const MARKET_PAGE_MAX: usize = 256;

/// The default rows one market page renders.
pub const MARKET_PAGE: usize = 20;

/// `market:catalog:{pack}:{code}`.
#[must_use]
pub fn catalog_key(pack: &str, code: &str) -> SmolStr {
    SmolStr::new(format!("market:catalog:{pack}:{code}"))
}
/// `market:listing:{id}`.
#[must_use]
pub fn listing_key(id: ListingId) -> SmolStr {
    SmolStr::new(format!("market:listing:{}", id.0))
}
/// `market:quote:{id}`.
#[must_use]
pub fn quote_key(id: QuoteId) -> SmolStr {
    SmolStr::new(format!("market:quote:{}", id.0))
}
/// `market:vendor:{id}`.
#[must_use]
pub fn vendor_key(id: VendorId) -> SmolStr {
    SmolStr::new(format!("market:vendor:{}", id.0))
}
/// `market:source:{id}`.
#[must_use]
pub fn source_key(id: MarketSourceId) -> SmolStr {
    SmolStr::new(format!("market:source:{}", id.0))
}

/// The in-RAM market slot — the single runtime source of truth.
#[derive(Clone, Debug, Default)]
pub struct MarketStore {
    /// The three wiring-frozen provider tables — content packs, availability engines
    /// and delivery. Frozen at composition and never written again, so "who could
    /// sell you this, and at what odds" is not a function of when you asked.
    pub(crate) tables: crate::module::MarketTables,
    pub(crate) entries: BTreeMap<SmolStr, CatalogEntry>,
    /// **catalogue reference → entry slug**, re-derived on every write: one live entry
    /// per `CatalogRef` across every pack, so "what is this" has one answer.
    pub(crate) by_code: BTreeMap<CatalogRef, SmolStr>,
    pub(crate) listings: BTreeMap<ListingId, Listing>,
    /// The ids of every listing still open — the caps' and the page's working set.
    pub(crate) live: BTreeSet<ListingId>,
    /// **(catalog, market) → listings**, the projection's own walk (filter, then take).
    pub(crate) by_pair: BTreeMap<(CatalogRef, MarketId), BTreeSet<ListingId>>,
    /// **vendor → listings**, what `MAX_LISTINGS_PER_VENDOR` bounds.
    pub(crate) by_vendor: BTreeMap<VendorId, BTreeSet<ListingId>>,
    /// **E7r2 P1 — `(inventory endpoint, commodity)` → the live lot listings on it.**
    ///
    /// The one-live-listing-per-lot index. It is a *re-derived projection* like every
    /// other index here — [`MarketStore::reindex`] is the only place it is built and
    /// [`MarketStore::indexes_agree`] says so out loud — and it carries only listings
    /// that are still live, because a sold-out one holds nothing.
    pub(crate) by_lot_pool: BTreeMap<(mush_inventory::EndpointRef, SmolStr), BTreeSet<ListingId>>,
    pub(crate) quotes: BTreeMap<QuoteId, Quote>,
    /// **buyer → quotes**, so the per-buyer cap is the buyer's own walk.
    ///
    /// Keyed by `mush_economy::principal_token` because `PrincipalId` is
    /// `Hash`-but-not-`Ord` — the same reason E6b's `with_party_caps` table is.
    pub(crate) by_buyer: BTreeMap<SmolStr, BTreeSet<QuoteId>>,
    /// **seller → quotes** (doc 18 cut IC10), keyed exactly as `by_buyer` is: a sale
    /// into a standing bid is a quote whose seller is the player, so the deliver-leg
    /// projection and the over-selling check walk the seller's own quotes.
    pub(crate) by_seller: BTreeMap<SmolStr, BTreeSet<QuoteId>>,
    /// **listing → quotes**, so a re-priced listing finds its outstanding quotes.
    pub(crate) by_listing: BTreeMap<ListingId, BTreeSet<QuoteId>>,
    /// **contract → quote**, so the settlement observer answers in O(log n).
    pub(crate) by_quote_contract: BTreeMap<ContractId, QuoteId>,
    /// **(catalog, market) → settled quotes** — the trade tape the projection folds.
    pub(crate) trades: BTreeMap<(CatalogRef, MarketId), BTreeSet<QuoteId>>,
    pub(crate) vendors: BTreeMap<VendorId, Vendor>,
    /// **organization → vendor**: one vendor per org, so "which one did I buy from"
    /// is a question about the world and not about ids.
    pub(crate) by_org: BTreeMap<OrgId, VendorId>,
    pub(crate) sources: BTreeMap<MarketSourceId, MarketSource>,
    pub(crate) next_listing: u64,
    pub(crate) next_quote: u64,
    pub(crate) next_vendor: u64,
    pub(crate) next_source: u64,
    /// Doc 18 cut IC7 item 7: the draw mirror's round-robin cursor — the last quote it
    /// visited. RAM only and never persisted (IC5's rotation idiom): a reboot starts the
    /// rotation again from the lowest id, which costs one lap and loses nothing.
    pub(crate) draw_cursor: Option<QuoteId>,
    /// Doc 18 cut IC14 row 9 (lead ruling L4): the sales into bids whose settlement the
    /// economy healed and never announced, stamped by this boot's
    /// `reconcile_contracts` and not yet told to the proceeds policy. RAM only and
    /// never persisted; drained at the top of the first sweep after boot (the game's
    /// store is hydrated by then). Bounded by `MAX_RECONCILED_QUOTES`, the reconcile's
    /// own walk. A crash between the boot and that sweep loses the fold: the narrow
    /// residue of the old gap (the quote is stamped, so the next boot does not see it
    /// again).
    pub(crate) healed_sales: Vec<QuoteId>,
    /// Doc 18 cut IC14 row 16: the sweep each kept lapsed hold-backed listing's
    /// `hold_standing` was last read on, by listing. RAM only; retained each sweep to
    /// the lapsed set, so bounded by `MAX_LISTINGS`.
    pub(crate) hold_checked: BTreeMap<ListingId, u64>,
    /// Doc 18 cut IC14 row 16: the sweep number each lapsed listing's holder release
    /// was staged at (`lot::release_lapsed_hold`). For `HOLD_RECHECK_SWEEPS` sweeps
    /// after it the listing's hold is read on every sweep, so a release that lands is
    /// seen on the next sweep rather than after the backoff. RAM only; retained with
    /// `hold_checked`, so bounded the same way.
    pub(crate) release_staged: BTreeMap<ListingId, u64>,
    /// Doc 18 cut IC14 row 16: the market sweeps run this boot (RAM only).
    pub(crate) sweeps: u64,
    /// Doc 18 cut IC14 row 16: how many `hold_standing` reads the sweep made this boot
    /// (a counter for tests, [`MarketStore::hold_reads`]).
    pub(crate) hold_reads: u64,
}

impl MarketStore {
    /// How many times the sweep read a kept lapsed hold-backed listing's standing this
    /// boot (doc 18 cut IC14 row 16): at most once per [`crate::HOLD_RECHECK_SWEEPS`]
    /// sweeps per listing.
    #[must_use]
    pub fn hold_reads(&self) -> u64 {
        self.hold_reads
    }

    // ---- the frozen wiring ------------------------------------------------

    /// **The system principal a stocked asset's title is minted under, when the
    /// composition named one** (WS04-S7).
    ///
    /// `None` is the fail-closed default and the honest answer for a server that
    /// composed no ruleset economy: nothing may be stocked, and
    /// `@market/vendor stock` says so in the market's own words rather than letting
    /// `mush_economy::issue_title` answer `Unauthorized` for a name the operator
    /// never chose. It is exposed because the composition root's two halves — this
    /// name and `EconomyModule::with_title_issuers` — must agree, and a
    /// `mush-server` test has no other seam to say so through:
    /// `crate::module::MarketTables` and its `issuer` field are `pub(crate)`.
    #[must_use]
    pub fn issuer(&self) -> Option<&mush_org::SystemPrincipalId> {
        self.tables.issuer.as_ref()
    }

    // ---- catalog ----------------------------------------------------------

    /// One entry by its `{pack}:{code}` slug.
    #[must_use]
    pub fn entry(&self, slug: &str) -> Option<&CatalogEntry> {
        self.entries.get(slug)
    }
    /// The live entry for a catalogue reference, whichever pack defines it.
    #[must_use]
    pub fn entry_of(&self, catalog: &CatalogRef) -> Option<&CatalogEntry> {
        self.by_code
            .get(catalog)
            .and_then(|slug| self.entries.get(slug))
    }
    /// Every entry, in slug order.
    pub fn entries(&self) -> impl Iterator<Item = &CatalogEntry> {
        self.entries.values()
    }
    /// Every entry of one pack, in slug order.
    pub fn entries_of_pack<'a>(&'a self, pack: &'a str) -> impl Iterator<Item = &'a CatalogEntry> {
        self.entries.values().filter(move |e| e.pack == pack)
    }
    /// Every pack name that has at least one entry.
    #[must_use]
    pub fn packs(&self) -> BTreeSet<SmolStr> {
        self.entries.values().map(|e| e.pack.clone()).collect()
    }

    // ---- listings ---------------------------------------------------------

    /// One listing.
    #[must_use]
    pub fn listing(&self, id: ListingId) -> Option<&Listing> {
        self.listings.get(&id)
    }
    /// Every open listing, newest first — the page's walk order.
    pub fn live_ids_desc(&self) -> impl Iterator<Item = ListingId> + '_ {
        self.live.iter().rev().copied()
    }
    /// Every open listing, in id order.
    pub fn live_listings(&self) -> impl Iterator<Item = &Listing> {
        self.live.iter().filter_map(|id| self.listings.get(id))
    }
    /// **The live lot listing over one `(endpoint, commodity)` pool, if there is one**
    /// (E7r2 P1, the one-live-listing-per-lot index).
    ///
    /// A point lookup on the re-derived index, never a walk: `@market/vendor stock`
    /// asks it before it publishes, and the sweep's reconcile arm asks it per listing.
    #[must_use]
    pub fn lot_listing_of(
        &self,
        endpoint: mush_inventory::EndpointRef,
        commodity: &mush_core::CanonicalId,
    ) -> Option<ListingId> {
        self.by_lot_pool
            .get(&(endpoint, SmolStr::new(commodity.as_str())))
            .and_then(|set| set.iter().next().copied())
    }

    /// **The Draw quotes in flight on one storefront pool** (doc 20 cut SM.12a; the rule
    /// tacsim's stock arm read since doc 18 cut IC8): `(undrawn, in_flight)` — Σ quantity
    /// over unsettled Draw quotes on `(endpoint, commodity)` whose goods have not yet
    /// been drawn, and whether any unsettled Draw quote is on it at all. The one copy:
    /// `restock_ask` and tacsim's `ask_reading` both read it.
    #[must_use]
    pub fn draw_reading(
        &self,
        endpoint: mush_inventory::EndpointRef,
        commodity: &mush_core::CanonicalId,
    ) -> (u64, bool) {
        let mut undrawn: u64 = 0;
        let mut in_flight = false;
        for quote in self.quotes() {
            let Some(lot) = &quote.lot else {
                continue;
            };
            if lot.kind != crate::record::PaidKind::Draw
                || lot.source.0 != endpoint
                || &lot.commodity.0 != commodity
                || quote.settled.is_some()
            {
                continue;
            }
            in_flight = true;
            if quote.drawn_at.is_none() {
                undrawn = undrawn.saturating_add(quote.quantity);
            }
        }
        (undrawn, in_flight)
    }

    /// **What matter a listing sells, and how it moves** (doc 18 §4.3, D-18-26; cut
    /// IC7, lead note 2b) — the one accessor, so which op a listing takes is never
    /// re-derived inline.
    ///
    /// `Supply::Lot` is [`LotMatter::Hold`]; `Supply::Stock { vault }` on an entry whose
    /// representation is `Representation::Lot` is [`LotMatter::Draw`] (commodity = the
    /// catalogue code, which must parse as a `CanonicalId`); everything else — E6c's
    /// counter listings — is `None`.
    #[must_use]
    pub fn lot_matter(&self, l: &Listing) -> Option<LotMatter> {
        match &l.source {
            Supply::Lot {
                endpoint,
                commodity,
                grade,
            } => Some(LotMatter::Hold {
                endpoint: endpoint.0,
                commodity: commodity.0.clone(),
                grade: *grade,
            }),
            Supply::Stock { vault } => {
                // Doc 18 cut IC14 row 14 (lead ruling L8): the kind frozen at
                // publication wins; only a pre-IC14 row (`None`) still derives it from
                // the live catalogue entry, until its next write stamps it.
                let draws = match l.draw {
                    Some(frozen) => frozen,
                    None => self.derived_draw(l),
                };
                if !draws {
                    return None;
                }
                let commodity = mush_core::CanonicalId::new(l.catalog.code.as_str()).ok()?;
                Some(LotMatter::Draw {
                    source: mush_inventory::EndpointRef(*vault),
                    commodity,
                })
            }
            // Doc 18 cut IC8: a standing bid sells nothing, so it moves no matter.
            Supply::Market { .. } | Supply::Procurement { .. } | Supply::Wanted { .. } => None,
        }
    }

    /// **Whether a listing would sell by Draw by IC7's live derivation** (doc 18 cut
    /// IC14 row 14): a `Supply::Stock` listing on an entry whose representation is
    /// `Representation::Lot`. Read only for a pre-IC14 listing (`draw: None`), by
    /// [`Self::lot_matter`] and by `put_listing` to stamp such a row on its first write.
    #[must_use]
    pub(crate) fn derived_draw(&self, l: &Listing) -> bool {
        matches!(l.source, Supply::Stock { .. })
            && self
                .entry_of(&l.catalog)
                .is_some_and(|entry| entry.representation == Representation::Lot)
    }

    /// **The live standing bid one vendor posts for one commodity, if any** (doc 18 cut
    /// IC8): what `post_wanted` refuses a second of. A walk of the vendor's own
    /// listings, bounded by `MAX_LISTINGS_PER_VENDOR`.
    #[must_use]
    pub fn wanted_of(
        &self,
        vendor: VendorId,
        commodity: &mush_core::CanonicalId,
    ) -> Option<ListingId> {
        self.by_vendor.get(&vendor).and_then(|set| {
            set.iter().copied().find(|id| {
                self.live.contains(id)
                    && self.listings.get(id).is_some_and(|l| {
                        matches!(&l.source, Supply::Wanted { commodity: c, .. } if &c.0 == commodity)
                    })
            })
        })
    }

    /// Every live **hold-backed** lot listing, for the sweep's hold reconcile arm.
    /// Bounded by the caller. A Draw listing holds no reservation, so it is not here.
    pub fn lot_listings(&self) -> impl Iterator<Item = &Listing> {
        self.live
            .iter()
            .filter_map(|id| self.listings.get(id))
            .filter(|l| l.source.lot().is_some())
    }

    /// How many listings this vendor holds open — what `MAX_LISTINGS_PER_VENDOR`
    /// bounds.
    #[must_use]
    pub fn listings_of_vendor(&self, vendor: VendorId) -> usize {
        self.by_vendor.get(&vendor).map_or(0, BTreeSet::len)
    }
    /// The open listings of one `(catalog, market)` pair, in id order — the
    /// projection's own walk, filtered before it is taken.
    pub fn listings_for<'a>(
        &'a self,
        catalog: &CatalogRef,
        market: &MarketId,
    ) -> impl Iterator<Item = &'a Listing> {
        self.by_pair
            .get(&(catalog.clone(), market.clone()))
            .into_iter()
            .flat_map(|set| set.iter().filter_map(|id| self.listings.get(id)))
    }

    /// Every venue this catalogue has **listings** in, in venue order — a `range` over
    /// the pair index scoped to the one catalogue (independent verify fold, D8).
    ///
    /// The whole point is that it is not a walk of `listings`: a good with one listing
    /// costs one record however many other catalogues the store holds.
    pub(crate) fn markets_of_catalog<'a>(
        &'a self,
        catalog: &'a CatalogRef,
    ) -> impl Iterator<Item = &'a MarketId> {
        self.by_pair
            .range((catalog.clone(), MarketId::floor())..)
            .take_while(move |((c, _), _)| c == catalog)
            .map(|((_, m), _)| m)
    }

    /// Every venue this catalogue has **trade-tape rows** in, in venue order — the
    /// same range over the tape index.
    pub(crate) fn markets_of_trades<'a>(
        &'a self,
        catalog: &'a CatalogRef,
    ) -> impl Iterator<Item = &'a MarketId> {
        self.trades
            .range((catalog.clone(), MarketId::floor())..)
            .take_while(move |((c, _), _)| c == catalog)
            .map(|((_, m), _)| m)
    }

    // ---- quotes -----------------------------------------------------------

    /// One quote.
    #[must_use]
    pub fn quote(&self, id: QuoteId) -> Option<&Quote> {
        self.quotes.get(&id)
    }
    /// Every quote, in id order.
    pub fn quotes(&self) -> impl DoubleEndedIterator<Item = &Quote> {
        self.quotes.values()
    }
    /// Every quote this buyer holds, in id order.
    pub fn quotes_of_buyer<'a>(&'a self, buyer: &PrincipalId) -> impl Iterator<Item = &'a Quote> {
        self.by_buyer
            .get(&mush_economy::principal_token(buyer))
            .into_iter()
            .flat_map(|set| set.iter().filter_map(|id| self.quotes.get(id)))
    }
    /// Every quote this principal is the **seller** of, in id order (doc 18 cut IC10:
    /// the player's sales into standing bids, and a vendor's own sales).
    pub fn quotes_of_seller<'a>(&'a self, seller: &PrincipalId) -> impl Iterator<Item = &'a Quote> {
        self.by_seller
            .get(&mush_economy::principal_token(seller))
            .into_iter()
            .flat_map(|set| set.iter().filter_map(|id| self.quotes.get(id)))
    }
    /// Every quote asked against one listing, in id order.
    pub fn quotes_of_listing(&self, listing: ListingId) -> impl Iterator<Item = &Quote> {
        self.by_listing
            .get(&listing)
            .into_iter()
            .flat_map(|set| set.iter().filter_map(|id| self.quotes.get(id)))
    }
    /// The quote a contract belongs to, if any.
    #[must_use]
    pub fn quote_of_contract(&self, contract: ContractId) -> Option<QuoteId> {
        self.by_quote_contract.get(&contract).copied()
    }
    /// The **settled** quotes of one `(catalog, market)` pair — the trade tape.
    pub fn trades_for<'a>(
        &'a self,
        catalog: &CatalogRef,
        market: &MarketId,
    ) -> impl Iterator<Item = &'a Quote> {
        self.trades
            .get(&(catalog.clone(), market.clone()))
            .into_iter()
            .flat_map(|set| set.iter().filter_map(|id| self.quotes.get(id)))
    }

    // ---- vendors and makers -----------------------------------------------

    /// One vendor.
    #[must_use]
    pub fn vendor(&self, id: VendorId) -> Option<&Vendor> {
        self.vendors.get(&id)
    }
    /// Every vendor, in id order.
    pub fn vendors(&self) -> impl Iterator<Item = &Vendor> {
        self.vendors.values()
    }
    /// The vendor an organization is, if it is one.
    #[must_use]
    pub fn vendor_of_org(&self, org: OrgId) -> Option<&Vendor> {
        self.by_org.get(&org).and_then(|id| self.vendors.get(id))
    }
    /// One market maker.
    #[must_use]
    pub fn source(&self, id: MarketSourceId) -> Option<&MarketSource> {
        self.sources.get(&id)
    }
    /// Every market maker, in id order.
    pub fn sources(&self) -> impl Iterator<Item = &MarketSource> {
        self.sources.values()
    }

    // ---- allocators -------------------------------------------------------

    /// Mint the next listing id, or `None` at the ceiling (the E4b1 B2 rule: an
    /// exhausted allocator refuses one operation loudly and never wraps a live id).
    pub fn alloc_listing(&mut self) -> Option<ListingId> {
        let id = ListingId(self.next_listing);
        self.next_listing = self.next_listing.checked_add(1)?;
        Some(id)
    }
    /// Mint the next quote id, or `None` at the ceiling.
    pub fn alloc_quote(&mut self) -> Option<QuoteId> {
        let id = QuoteId(self.next_quote);
        self.next_quote = self.next_quote.checked_add(1)?;
        Some(id)
    }
    /// Mint the next vendor id, or `None` at the ceiling.
    pub fn alloc_vendor(&mut self) -> Option<VendorId> {
        let id = VendorId(self.next_vendor);
        self.next_vendor = self.next_vendor.checked_add(1)?;
        Some(id)
    }
    /// Mint the next maker id, or `None` at the ceiling.
    pub fn alloc_source(&mut self) -> Option<MarketSourceId> {
        let id = MarketSourceId(self.next_source);
        self.next_source = self.next_source.checked_add(1)?;
        Some(id)
    }

    /// The allocator document.
    #[must_use]
    pub fn meta_doc(&self) -> serde_json::Value {
        serde_json::json!({
            "next_listing_id": self.next_listing,
            "next_quote_id": self.next_quote,
            "next_vendor_id": self.next_vendor,
            "next_source_id": self.next_source,
        })
    }

    /// Re-derive **every** index from the records — the one place they are built, so
    /// they cannot drift.
    pub(crate) fn reindex(&mut self) {
        self.by_code = self
            .entries
            .iter()
            .map(|(slug, e)| (e.catalog.clone(), slug.clone()))
            .collect();
        // Doc 18 cut IC10: an exhausted bid stands at quantity 0 and is still open — it
        // is listed (" — not buying"), it blocks a second bid for the same good, and a
        // retire withdraws it. Every other listing at 0 is closed.
        self.live = self
            .listings
            .values()
            .filter(|l| l.quantity > 0 || l.source.is_wanted())
            .map(|l| l.id)
            .collect();
        self.by_pair = BTreeMap::new();
        self.by_vendor = BTreeMap::new();
        // E7r2 P1: only a LIVE lot listing holds a pool, so only a live one is
        // indexed — a sold-out row has already had its reservation consumed. Doc 18 cut
        // IC7: a Draw listing (`Supply::Stock` on a lot entry) sits on its storefront
        // pool too, so the one-live-listing-per-pool rule covers both kinds; the pools
        // are resolved first because `lot_matter` reads the catalogue index above.
        let pools: Vec<(mush_inventory::EndpointRef, SmolStr, ListingId)> = self
            .listings
            .values()
            .filter(|l| l.quantity > 0)
            .filter_map(|l| {
                self.lot_matter(l)
                    .map(|m| (m.source(), SmolStr::new(m.commodity().as_str()), l.id))
            })
            .collect();
        self.by_lot_pool = BTreeMap::new();
        for l in self.listings.values() {
            self.by_pair
                .entry((l.catalog.clone(), l.market.clone()))
                .or_default()
                .insert(l.id);
            self.by_vendor.entry(l.vendor).or_default().insert(l.id);
        }
        for (endpoint, commodity, id) in pools {
            self.by_lot_pool
                .entry((endpoint, commodity))
                .or_default()
                .insert(id);
        }
        self.by_buyer = BTreeMap::new();
        self.by_seller = BTreeMap::new();
        self.by_listing = BTreeMap::new();
        self.by_quote_contract = BTreeMap::new();
        self.trades = BTreeMap::new();
        for q in self.quotes.values() {
            self.by_buyer
                .entry(mush_economy::principal_token(&q.buyer))
                .or_default()
                .insert(q.id);
            self.by_seller
                .entry(mush_economy::principal_token(&q.seller))
                .or_default()
                .insert(q.id);
            self.by_listing.entry(q.listing).or_default().insert(q.id);
            if let Some(c) = q.contract {
                self.by_quote_contract.insert(c, q.id);
            }
            if q.is_trade() {
                self.trades
                    .entry((q.catalog.clone(), q.market.clone()))
                    .or_default()
                    .insert(q.id);
            }
        }
        self.by_org = self.vendors.values().map(|v| (v.org, v.id)).collect();
    }

    /// Whether every derived index agrees with the records.
    ///
    /// A test-and-`debug_assert` predicate, not a runtime gate: the indexes are
    /// re-derived by `MarketStore::reindex` on every write path and at boot, and
    /// this is what says so out loud (E6a verify fold D7's shape).
    #[must_use]
    pub fn indexes_agree(&self) -> bool {
        let mut probe = self.clone();
        probe.reindex();
        probe.by_code == self.by_code
            && probe.live == self.live
            && probe.by_pair == self.by_pair
            && probe.by_vendor == self.by_vendor
            && probe.by_lot_pool == self.by_lot_pool
            && probe.by_buyer == self.by_buyer
            && probe.by_seller == self.by_seller
            && probe.by_listing == self.by_listing
            && probe.by_quote_contract == self.by_quote_contract
            && probe.trades == self.trades
            && probe.by_org == self.by_org
    }

    /// The trade-tape rows of a pair that are past [`MAX_TRADES_PER_PAIR`], **oldest
    /// settlement first** — what a fresh trade evicts.
    ///
    /// Ordered by `Settlement.at`, not by `QuoteId` (independent verify fold, R10):
    /// a quote issued early and settled late used to evict rows that had settled
    /// before it, so "the window of the last 32 trades" the projection folds could drop
    /// the most recent settlement. The id is the tiebreaker, so the order is total and
    /// the eviction is deterministic across a reboot.
    #[must_use]
    pub(crate) fn overflow_trades(&self, catalog: &CatalogRef, market: &MarketId) -> Vec<QuoteId> {
        let Some(set) = self.trades.get(&(catalog.clone(), market.clone())) else {
            return Vec::new();
        };
        let over = set.len().saturating_sub(MAX_TRADES_PER_PAIR);
        if over == 0 {
            return Vec::new();
        }
        let mut rows: Vec<(i64, QuoteId)> = set
            .iter()
            .filter_map(|id| {
                self.quotes
                    .get(id)
                    .map(|q| (q.settled.map_or(0, |s| s.at), *id))
            })
            .collect();
        rows.sort_unstable();
        rows.into_iter().take(over).map(|(_, id)| id).collect()
    }

    /// How many settled trade-tape rows the store holds, across every pair — what
    /// [`MAX_TRADE_ROWS`] bounds and what [`MAX_QUOTES`] no longer has to.
    #[must_use]
    pub fn trade_rows(&self) -> usize {
        self.quotes.values().filter(|q| q.is_trade()).count()
    }

    /// How many **open** offers the store holds — the count `trade::quote` checks
    /// against [`MAX_QUOTES`], so a full tape can never refuse a new quote
    /// (independent verify fold, R4).
    #[must_use]
    pub fn open_quotes(&self) -> usize {
        self.quotes.values().filter(|q| !q.is_trade()).count()
    }

    /// The trade-tape rows past [`MAX_TRADE_ROWS`] **across every pair**, oldest
    /// settlement first.
    #[must_use]
    pub(crate) fn overflow_tape(&self) -> Vec<QuoteId> {
        let mut rows: Vec<(i64, QuoteId)> = self
            .quotes
            .values()
            .filter_map(|q| q.settled.map(|s| (s.at, q.id)))
            .collect();
        let over = rows.len().saturating_sub(MAX_TRADE_ROWS);
        if over == 0 {
            return Vec::new();
        }
        rows.sort_unstable();
        rows.into_iter().take(over).map(|(_, id)| id).collect()
    }

    /// How many records one projection of this pair would **visit**.
    ///
    /// The honest statement of the bound, and deliberately **not** a field on
    /// [`crate::Projection`]: the count is a function of hidden state (a hidden listing
    /// is visited and then dropped), so publishing it inside the value the gate
    /// compares byte for byte would itself be the leak. The projection's `take` bounds
    /// the levels it *matches*; what bounds the records it *touches* is the store's own
    /// [`MAX_LISTINGS`] / [`MAX_QUOTES`] caps, and this is the tripwire that says so.
    #[must_use]
    pub fn projection_visits(&self, catalog: &CatalogRef, market: &MarketId) -> usize {
        self.listings_for(catalog, market)
            .count()
            .saturating_add(self.trades_for(catalog, market).count())
    }
}

// ---- write paths -----------------------------------------------------------

/// Queue one module write.
pub(crate) fn push_op(out: &mut Outbox, op: ModuleWrite) {
    out.defer(mush_core::Deferred::Persist(ModuleOp {
        ns: SmolStr::new_static(NS),
        op,
    }));
}

macro_rules! put_record {
    ($fn_name:ident, $ty:ty, $field:ident, $key:expr, $what:literal) => {
        /// Install a record into RAM, re-derive the indexes and queue its document —
        /// the single write path.
        pub(crate) fn $fn_name(ctx: &mut Ctx<'_>, r: $ty) {
            let key = $key(&r);
            if let Some(store) = ctx.w.module_state_mut::<MarketStore>() {
                store.$field.insert(key.0, r.clone());
                store.reindex();
            }
            let Ok(value) = serde_json::to_value(&r) else {
                tracing::error!(concat!("market: a ", $what, " did not serialize"));
                return;
            };
            push_op(ctx.out, ModuleWrite::JsonSet { key: key.1, value });
        }
    };
}

/// Install a listing into RAM, re-derive the indexes and queue its document — the
/// single write path. Doc 18 cut IC14 row 14: a listing written before IC14 (`draw:
/// None`) is stamped here with the kind IC7's live derivation gives at this moment, so
/// every re-save (a revision bump at activation, a re-post, a release) freezes it and it
/// is never re-derived after.
pub(crate) fn put_listing(ctx: &mut Ctx<'_>, mut l: Listing) {
    if l.draw.is_none() {
        let derived = ctx
            .w
            .module_state::<MarketStore>()
            .is_some_and(|store| store.derived_draw(&l));
        l.draw = Some(derived);
    }
    put_listing_record(ctx, l);
}

put_record!(
    put_listing_record,
    Listing,
    listings,
    |l: &Listing| (l.id, listing_key(l.id)),
    "listing"
);
put_record!(
    put_vendor,
    Vendor,
    vendors,
    |v: &Vendor| (v.id, vendor_key(v.id)),
    "vendor"
);
put_record!(
    put_source,
    MarketSource,
    sources,
    |s: &MarketSource| (s.id, source_key(s.id)),
    "market maker"
);

/// Install a quote and queue its document — the single write path, and the one place
/// the trade tape is trimmed.
pub(crate) fn put_quote(ctx: &mut Ctx<'_>, q: Quote) {
    let (catalog, market, trade) = (q.catalog.clone(), q.market.clone(), q.is_trade());
    if let Some(store) = ctx.w.module_state_mut::<MarketStore>() {
        store.quotes.insert(q.id, q.clone());
        store.reindex();
    }
    let Ok(value) = serde_json::to_value(&q) else {
        tracing::error!(quote = q.id.0, "market: a quote did not serialize");
        return;
    };
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: quote_key(q.id),
            value,
        },
    );
    if trade {
        // Two ceilings, not one (independent verify fold, R4): the pair's own window,
        // and the tape's global one — because the per-pair trim only ever fires for the
        // pair a new trade landed in, so without the second a store could fill with
        // rows no future trade would ever evict.
        let evict = ctx
            .w
            .module_state::<MarketStore>()
            .map(|s| {
                let mut ids = s.overflow_trades(&catalog, &market);
                ids.extend(s.overflow_tape());
                ids
            })
            .unwrap_or_default();
        for id in evict {
            del_quote(ctx, id);
        }
    }
}

/// Install a catalog entry and queue its document — the single write path.
pub(crate) fn put_entry(ctx: &mut Ctx<'_>, e: CatalogEntry) {
    let slug = SmolStr::new(e.slug());
    if let Some(store) = ctx.w.module_state_mut::<MarketStore>() {
        store.entries.insert(slug.clone(), e.clone());
        store.reindex();
    }
    let Ok(value) = serde_json::to_value(&e) else {
        tracing::error!(%slug, "market: a catalog entry did not serialize");
        return;
    };
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: catalog_key(&e.pack, &e.catalog.code),
            value,
        },
    );
}

/// Forget a listing: out of RAM, out of the indexes, out of the store.
///
/// The money was never here — it is the contract's, and the contract keeps its own
/// terminal record — so a deleted listing costs a market row and nothing else. That
/// is the trade E6a's `del_run` and E6b's `del_quote` already make, and it is what
/// keeps a list/withdraw loop from minting permanent documents.
pub(crate) fn del_listing(ctx: &mut Ctx<'_>, id: ListingId) {
    if let Some(store) = ctx.w.module_state_mut::<MarketStore>() {
        store.listings.remove(&id);
        store.reindex();
    }
    push_op(
        ctx.out,
        ModuleWrite::Del {
            key: listing_key(id),
        },
    );
}

/// Forget a quote — a lapsed offer, or a trade tape row the window rolled past.
pub(crate) fn del_quote(ctx: &mut Ctx<'_>, id: QuoteId) {
    if let Some(store) = ctx.w.module_state_mut::<MarketStore>() {
        store.quotes.remove(&id);
        store.reindex();
    }
    push_op(ctx.out, ModuleWrite::Del { key: quote_key(id) });
}

/// Forget a catalog entry (a pack reload that dropped it).
pub(crate) fn del_entry(ctx: &mut Ctx<'_>, pack: &str, code: &str) {
    let slug = SmolStr::new(format!("{pack}:{code}"));
    if let Some(store) = ctx.w.module_state_mut::<MarketStore>() {
        store.entries.remove(&slug);
        store.reindex();
    }
    push_op(
        ctx.out,
        ModuleWrite::Del {
            key: catalog_key(pack, code),
        },
    );
}

/// Queue the allocator document.
pub(crate) fn persist_meta(ctx: &mut Ctx<'_>) {
    let Some(meta) = ctx
        .w
        .module_state::<MarketStore>()
        .map(MarketStore::meta_doc)
    else {
        return;
    };
    push_op(
        ctx.out,
        ModuleWrite::JsonSet {
            key: SmolStr::new_static(META_KEY),
            value: meta,
        },
    );
}

// ---- boot ------------------------------------------------------------------

/// Build the slot from the module's boot documents.
///
/// **Total**: every document it cannot use is skipped loudly and the world boots. The
/// allocator floors come from every document's parsed **key**, loaded or not, so a
/// corrupt `market:listing:9` plus a lost `market:meta` can never re-mint id 9 and
/// `JsonSet` straight over the evidence (E6b verify fold, R8).
#[must_use]
pub fn load(kv: &[(SmolStr, serde_json::Value)]) -> MarketStore {
    let mut store = MarketStore::default();
    let mut seen = [0u64; 4];
    for (key, value) in kv {
        let key = key.as_str();
        if let Some(rest) = key.strip_prefix("market:catalog:") {
            load_entry(&mut store, key, rest, value);
        } else if let Some(rest) = key.strip_prefix("market:listing:") {
            claim(&mut seen[0], rest);
            load_one(
                &mut store,
                key,
                rest,
                value,
                "listing",
                MAX_LISTINGS,
                |s| s.listings.len(),
                |s, id, r: Listing| {
                    if r.id != ListingId(id) {
                        return false;
                    }
                    s.listings.insert(ListingId(id), r);
                    true
                },
            );
        } else if let Some(rest) = key.strip_prefix("market:quote:") {
            claim(&mut seen[1], rest);
            load_one(
                &mut store,
                key,
                rest,
                value,
                "quote",
                MAX_QUOTES,
                |s| s.quotes.len(),
                |s, id, r: Quote| {
                    if r.id != QuoteId(id) {
                        return false;
                    }
                    s.quotes.insert(QuoteId(id), r);
                    true
                },
            );
        } else if let Some(rest) = key.strip_prefix("market:vendor:") {
            claim(&mut seen[2], rest);
            load_one(
                &mut store,
                key,
                rest,
                value,
                "vendor",
                MAX_VENDORS,
                |s| s.vendors.len(),
                |s, id, r: Vendor| {
                    if r.id != VendorId(id) {
                        return false;
                    }
                    s.vendors.insert(VendorId(id), r);
                    true
                },
            );
        } else if let Some(rest) = key.strip_prefix("market:source:") {
            claim(&mut seen[3], rest);
            load_one(
                &mut store,
                key,
                rest,
                value,
                "market maker",
                MAX_SOURCES,
                |s| s.sources.len(),
                |s, id, r: MarketSource| {
                    if r.id != MarketSourceId(id) {
                        return false;
                    }
                    s.sources.insert(MarketSourceId(id), r);
                    true
                },
            );
        } else if key == META_KEY {
            store.next_listing = read_u64(value, "next_listing_id");
            store.next_quote = read_u64(value, "next_quote_id");
            store.next_vendor = read_u64(value, "next_vendor_id");
            store.next_source = read_u64(value, "next_source_id");
        }
        // Every other key belongs to a forward schema; tolerated in both directions.
    }
    store.next_listing = store
        .next_listing
        .max(floor(store.listings.keys().map(|i| i.0)))
        .max(seen[0]);
    store.next_quote = store
        .next_quote
        .max(floor(store.quotes.keys().map(|i| i.0)))
        .max(seen[1]);
    store.next_vendor = store
        .next_vendor
        .max(floor(store.vendors.keys().map(|i| i.0)))
        .max(seen[2]);
    store.next_source = store
        .next_source
        .max(floor(store.sources.keys().map(|i| i.0)))
        .max(seen[3]);
    store.reindex();
    trim_on_load(&mut store);
    store
}

/// Re-run the **per-vendor** and **per-pair** caps on load, not merely on write
/// (independent verify fold, R8 and R10).
///
/// `load_one` enforces only the whole-store ceilings, so a store carrying more than
/// [`MAX_LISTINGS_PER_VENDOR`] listings for one vendor — an operator lowering the
/// constant, a hand-repaired document, a future migration — used to come back live and
/// keep publishing under a shop `retire_vendor` had already emptied; and a lost `Del`
/// from a tape eviction left a pair above [`MAX_TRADES_PER_PAIR`] until the next trade
/// in that very pair happened to trim it. Both are skipped **loudly** here, in the same
/// direction every other load loss takes: the money is the economy's and survives, and
/// the surplus record is one a vendor re-creates.
fn trim_on_load(store: &mut MarketStore) {
    let over_vendor: Vec<ListingId> = store
        .by_vendor
        .values()
        .flat_map(|ids| {
            ids.iter()
                .rev()
                .take(ids.len().saturating_sub(MAX_LISTINGS_PER_VENDOR))
        })
        .copied()
        .collect();
    for id in over_vendor {
        tracing::warn!(
            listing = id.0,
            "market: past the per-vendor listing cap on load; skipped"
        );
        store.listings.remove(&id);
    }
    let pairs: Vec<(CatalogRef, MarketId)> = store.trades.keys().cloned().collect();
    for (catalog, market) in pairs {
        for id in store.overflow_trades(&catalog, &market) {
            tracing::warn!(
                quote = id.0,
                "market: past the per-pair trade-tape cap on load; skipped"
            );
            store.quotes.remove(&id);
        }
    }
    for id in store.overflow_tape() {
        tracing::warn!(
            quote = id.0,
            "market: past the whole-tape cap on load; skipped"
        );
        store.quotes.remove(&id);
    }
    store.reindex();
}

fn read_u64(value: &serde_json::Value, field: &str) -> u64 {
    value
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

fn floor(ids: impl Iterator<Item = u64>) -> u64 {
    ids.map(|n| n.saturating_add(1)).max().unwrap_or(0)
}

/// Record an id claim from a **parsed key**, loaded or not (R8's rule).
fn claim(slot: &mut u64, rest: &str) {
    if let Ok(n) = rest.parse::<u64>() {
        *slot = (*slot).max(n.saturating_add(1));
    }
}

/// The one loader body every numeric family shares: parse the key, enforce the
/// whole-store cap on **load** as well as on write (E6b verify fold, D6), decode
/// through the record's own `TryFrom` wire, and apply the identity rule (DL-07) —
/// a document that disagrees with its own key is dropped loudly, never trusted.
#[allow(clippy::too_many_arguments)]
fn load_one<T, C, I>(
    store: &mut MarketStore,
    key: &str,
    rest: &str,
    value: &serde_json::Value,
    what: &str,
    cap: usize,
    count: C,
    insert: I,
) where
    T: serde::de::DeserializeOwned,
    C: Fn(&MarketStore) -> usize,
    I: Fn(&mut MarketStore, u64, T) -> bool,
{
    let Ok(n) = rest.parse::<u64>() else {
        tracing::warn!(%key, %what, "market: a document has an unreadable key; skipped");
        return;
    };
    if count(store) >= cap {
        tracing::warn!(%key, %what, "market: past the store cap; skipped");
        return;
    }
    match serde_json::from_value::<T>(value.clone()) {
        Ok(r) => {
            if !insert(store, n, r) {
                tracing::warn!(
                    %key, %what,
                    "market: a document disagrees with its own key; skipped"
                );
            }
        }
        Err(error) => {
            tracing::warn!(%key, %what, %error, "market: a document did not parse; skipped");
        }
    }
}

/// Load one `market:catalog:{pack}:{code}` document.
fn load_entry(store: &mut MarketStore, key: &str, rest: &str, value: &serde_json::Value) {
    let Some((pack, code)) = rest.split_once(':') else {
        tracing::warn!(%key, "market: a catalog document has an unreadable key; skipped");
        return;
    };
    if store.entries.len() >= MAX_CATALOG_ENTRIES {
        tracing::warn!(%key, "market: past the catalog cap; skipped");
        return;
    }
    match serde_json::from_value::<CatalogEntry>(value.clone()) {
        // The identity rule: the record must agree with its own key on both halves.
        Ok(e) if e.pack == pack && e.catalog.code == code => {
            store.entries.insert(SmolStr::new(e.slug()), e);
        }
        Ok(_) => {
            tracing::warn!(%key, "market: a catalog document disagrees with its own key; skipped");
        }
        Err(error) => {
            tracing::warn!(%key, %error, "market: a catalog document did not parse; skipped");
        }
    }
}

/// A live world object that is not on its way out — what a listing's named object and
/// a delivery destination must be.
#[must_use]
pub fn is_live_object(w: &mush_core::World, id: ObjId) -> bool {
    w.get(id)
        .is_some_and(|o| !o.flags.contains(mush_core::Flags::GOING))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing_doc(id: u64) -> serde_json::Value {
        serde_json::json!({
            "id": id, "seller": { "Organization": 1 }, "vendor": 1, "market": "seattle",
            "catalog": { "ruleset": "sr", "code": "ammo" },
            "source": { "stock": { "vault": 1 } },
            "price": 10, "quantity": 5, "visibility": "public",
            "expires": { "clock": "RealUtc", "at": 10_000 }, "revision": 1
        })
    }

    #[test]
    fn the_allocator_floor_never_reissues_a_live_id() {
        let kv = vec![
            (SmolStr::new(META_KEY), serde_json::json!({})),
            (SmolStr::new("market:listing:9"), listing_doc(9)),
        ];
        let store = load(&kv);
        assert_eq!(store.listings.len(), 1);
        assert_eq!(store.next_listing, 10);
    }

    #[test]
    fn a_corrupt_document_still_claims_its_id_and_the_world_boots() {
        let kv = vec![
            (
                SmolStr::new("market:listing:4"),
                serde_json::json!({ "nope": true }),
            ),
            (
                SmolStr::new("market:vendor:2"),
                serde_json::json!("not even an object"),
            ),
        ];
        let store = load(&kv);
        assert!(store.listings.is_empty() && store.vendors.is_empty());
        assert_eq!(store.next_listing, 5, "a parsed key is a claim on its id");
        assert_eq!(store.next_vendor, 3);
    }

    #[test]
    fn a_document_that_disagrees_with_its_key_is_skipped() {
        let kv = vec![(SmolStr::new("market:listing:4"), listing_doc(9))];
        assert!(load(&kv).listings.is_empty());
    }

    #[test]
    fn a_corrupt_catalog_document_is_skipped_and_the_rest_of_the_pack_loads() {
        let good = serde_json::json!({
            "catalog": { "ruleset": "sr", "code": "ammo" },
            "pack": "sr.core", "definition_version": 1, "name": "Ammo",
            "representation": "instrument"
        });
        let mut wrong_key = good.clone();
        wrong_key["catalog"]["code"] = serde_json::json!("other");
        let kv = vec![
            (SmolStr::new("market:catalog:sr.core:ammo"), good),
            // A body that will not parse at all…
            (
                SmolStr::new("market:catalog:sr.core:broken"),
                serde_json::json!({ "nope": true }),
            ),
            // …a body that disagrees with its own key (the identity rule)…
            (SmolStr::new("market:catalog:sr.core:other2"), wrong_key),
            // …and a key with no `{pack}:{code}` shape at all.
            (
                SmolStr::new("market:catalog:nocolon"),
                serde_json::json!({ "nope": true }),
            ),
        ];
        let store = load(&kv);
        assert_eq!(store.entries().count(), 1, "the loss was not contained");
        assert!(store.entry("sr.core:ammo").is_some());
        assert!(store.indexes_agree());
    }

    #[test]
    fn every_index_is_re_derived_from_the_records() {
        let kv = vec![
            (SmolStr::new("market:listing:1"), listing_doc(1)),
            (SmolStr::new("market:listing:2"), listing_doc(2)),
        ];
        let store = load(&kv);
        assert!(store.indexes_agree());
        assert_eq!(store.live.len(), 2);
        assert_eq!(store.listings_of_vendor(VendorId(1)), 2);
    }

    // ---- E6c independent verify fold ----

    /// One settled trade-tape row's document, settling at `at`.
    fn trade_doc(id: u64, at: i64) -> serde_json::Value {
        serde_json::json!({
            "id": id, "listing": 1, "buyer": { "Object": 5 },
            "seller": { "Organization": 1 },
            "catalog": { "ruleset": "sr", "code": "ammo" }, "market": "seattle",
            "price": 10, "quantity": 1,
            "observed": { "definition_version": 1, "listing": 1, "artefact": null },
            "expires": { "clock": "RealUtc", "at": 10_000 },
            "contract": 7, "settled": { "transaction": 3, "at": at }, "revision": 1
        })
    }

    /// **R10.** The trade ring is a window of the last thirty-two **settlements**, not
    /// of the thirty-two lowest quote ids — a quote issued early and settled late used
    /// to evict rows that had settled before it — and the per-pair trim is re-run on
    /// **load**, so a lost `Del` from an eviction self-heals at boot instead of waiting
    /// for the next trade in that very pair.
    #[test]
    fn vf_r10_the_trade_ring_evicts_the_oldest_settlement_and_is_re_trimmed_on_load() {
        // The HIGHEST quote id settled FIRST, so id order and settlement order disagree
        // about every row.
        let kv: Vec<(SmolStr, serde_json::Value)> = (1..=MAX_TRADES_PER_PAIR as u64 + 1)
            .map(|id| {
                (
                    SmolStr::new(format!("market:quote:{id}")),
                    trade_doc(id, 1_000 - i64::try_from(id).unwrap_or(0)),
                )
            })
            .collect();
        let store = load(&kv);
        assert_eq!(
            store.quotes.len(),
            MAX_TRADES_PER_PAIR,
            "the per-pair cap was not re-run on load"
        );
        let evicted = QuoteId(MAX_TRADES_PER_PAIR as u64 + 1);
        assert!(
            store.quote(evicted).is_none(),
            "the ring evicted by quote id rather than by settlement time"
        );
        assert!(store.quote(QuoteId(1)).is_some(), "the newest row was lost");
        assert!(store.indexes_agree());
    }

    /// **R8.** The **per-vendor** cap is re-run on load, not merely on write: a store
    /// carrying more than `MAX_LISTINGS_PER_VENDOR` listings for one vendor used to come
    /// back whole, and `retire_vendor` — which took only the first 64 — left the surplus
    /// live, publishing band and depth under a shop that no longer exists.
    #[test]
    fn vf_r8_the_per_vendor_listing_cap_is_re_run_on_load() {
        let kv: Vec<(SmolStr, serde_json::Value)> = (1..=MAX_LISTINGS_PER_VENDOR as u64 + 8)
            .map(|id| {
                (
                    SmolStr::new(format!("market:listing:{id}")),
                    listing_doc(id),
                )
            })
            .collect();
        let store = load(&kv);
        assert_eq!(
            store.listings_of_vendor(VendorId(1)),
            MAX_LISTINGS_PER_VENDOR,
            "the per-vendor cap was enforced on write only"
        );
        assert!(store.indexes_agree());
        // The allocator floor still comes from every parsed key, loaded or not, so a
        // skipped surplus can never be re-minted over.
        assert_eq!(store.next_listing, MAX_LISTINGS_PER_VENDOR as u64 + 9);
    }

    /// **R4.** The tape has a ceiling of its own and it is **below** `MAX_QUOTES`, so a
    /// store full of history always leaves room for open offers: before, 64 traded pairs
    /// at `MAX_TRADES_PER_PAIR` filled the whole quote family and `+market/quote`
    /// answered "The market is busy" to every buyer for ever, with no new trade possible
    /// to evict anything.
    #[test]
    fn vf_r4_the_trade_tape_has_its_own_ceiling_below_the_quote_cap() {
        // (`MAX_TRADE_ROWS < MAX_QUOTES` is a `const` assertion in `record.rs`.)
        // One row per pair, so `MAX_TRADES_PER_PAIR` never fires and only the global
        // ceiling can trim: the exact shape that used to be unevictable.
        let kv: Vec<(SmolStr, serde_json::Value)> = (1..=MAX_TRADE_ROWS as u64 + 4)
            .map(|id| {
                let mut doc = trade_doc(id, 1_000 + i64::try_from(id).unwrap_or(0));
                doc["catalog"]["code"] = serde_json::json!(format!("ammo.{id}"));
                (SmolStr::new(format!("market:quote:{id}")), doc)
            })
            .collect();
        let store = load(&kv);
        assert_eq!(store.trade_rows(), MAX_TRADE_ROWS);
        assert_eq!(
            store.open_quotes(),
            0,
            "the tape and the open offers are counted apart"
        );
        // Oldest settlement first: ids 1..=4 settled earliest and are the four gone.
        for id in 1..=4u64 {
            assert!(store.quote(QuoteId(id)).is_none(), "row {id} survived");
        }
        assert!(store.indexes_agree());
    }

    #[test]
    fn ic10_indexes_agree_with_a_zero_bid_and_a_seller_quote() {
        let bid = serde_json::json!({
            "id": 4, "seller": { "Organization": 2 }, "vendor": 1, "market": "stickney",
            "catalog": { "ruleset": "tacsim", "code": "metals" },
            "source": { "wanted": { "vault": 9, "commodity": "metals", "min_grade": 0 } },
            "price": 28000, "quantity": 0, "expires": { "clock": "RealUtc", "at": 10_000 },
            "revision": 2
        });
        let quote = serde_json::json!({
            "id": 1, "listing": 4, "buyer": { "Organization": 2 }, "seller": { "Object": 5 },
            "catalog": { "ruleset": "tacsim", "code": "metals" }, "market": "stickney",
            "price": 28000, "quantity": 20,
            "observed": { "definition_version": 1, "listing": 1, "artefact": null },
            "expires": { "clock": "RealUtc", "at": 3_700 }, "contract": 3, "activated": true,
            "lot": { "kind": "deliver", "source": 11, "destination": 9, "commodity": "metals",
                     "grade": 0, "accepted_at": 100, "draw_by": 3_100,
                     "marker": "8000000000000064aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa" },
            "revision": 2
        });
        let kv = vec![
            (SmolStr::new("market:listing:4"), bid),
            (SmolStr::new("market:listing:5"), listing_doc(5)),
            (SmolStr::new("market:quote:1"), quote),
        ];
        let store = load(&kv);
        assert_eq!(
            store.listings.len(),
            2,
            "the zero bid and the ask both load"
        );
        assert!(store.indexes_agree());
        // The exhausted bid is still open: in `live`, found by `wanted_of`…
        assert!(store.live.contains(&ListingId(4)));
        assert!(store.live.contains(&ListingId(5)));
        let metals = mush_core::CanonicalId::new("metals").expect("id");
        assert_eq!(store.wanted_of(VendorId(1), &metals), Some(ListingId(4)));
        // …never in the pool index, and never published to band or depth.
        assert!(
            store
                .by_lot_pool
                .values()
                .all(|set| !set.contains(&ListingId(4)))
        );
        let zero = store.listing(ListingId(4)).expect("loaded");
        assert!(!store.publishes(zero, &[], 0));
        // The seller index finds the sale under the player, and the buyer index under
        // the house.
        let seller = PrincipalId::Object(ObjId(5));
        assert_eq!(
            store
                .quotes_of_seller(&seller)
                .map(|q| q.id)
                .collect::<Vec<_>>(),
            vec![QuoteId(1)]
        );
        assert_eq!(
            store
                .quotes_of_buyer(&PrincipalId::Organization(OrgId(2)))
                .count(),
            1
        );
    }
}

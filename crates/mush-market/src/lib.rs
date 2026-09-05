//! `mush-market` — the **neutral market** (E6c, `docs/design/08f` §5).
//!
//! Bounded catalogs and content packs, listings, buyer-as-issuer quotes with declared
//! expiry clocks, vendors from stock / market source / procurement, NPC market makers
//! with explicit budgets, and — first, because everything else could leak into it —
//! **the safe aggregate public projection**.
//!
//! ## Game-neutral, and that is a dependency claim
//!
//! `mush-market → {core, org, economy}` plus serde/serde_json/smol_str/tracing, and
//! **nothing else**: no `mush-party`, no `mush-canon`, no ruleset edge. Nothing in
//! this crate names a run, a fence, nuyen or a runner (answer 20 — it must serve WoD
//! and TacSim too). The economy never depends on the market (doc 08 D11, doc 10 §14),
//! so `cargo tree -p mush-market` is the test of both halves.
//!
//! Prices, availability, legality and what an item *means* are the ruleset's, and
//! they arrive through three wiring-frozen tables — [`CatalogSource`],
//! [`AvailabilityProvider`] and [`DeliveryProvider`] — never through an edge.
//!
//! ## The gate (§5.3, D-E6-14, doc 10 §14(b))
//!
//! *Two worlds differing only in state the viewer may not see produce byte-identical
//! public output.* [`MarketStore::projection`] takes **no viewer argument** — that
//! signature is the proof — and is folded only over the public set; last trade and
//! volume come from **settled journal transactions** re-checked against the economy's
//! own `ContractState::Settled`; depth is bucketed into [`DEPTH_BUCKETS`] levels and
//! never published per seller; and the whole per-viewer surface is the listing list,
//! through the existence-hiding renderer. [`crate::projection`]'s module header
//! states the five parts in full.
//!
//! ## What this crate does **not** do
//!
//! - **It reserves nothing.** A quote is a price with an expiry; the listing's
//!   quantity decrements at contract **activation** and nowhere else (OPEN 4). There
//!   is no reservation family and no new `Hold` field — 08i §4 rule 1's exemption
//!   does not cover reservations or mechanical offers, and doc 10 T4 keeps E4 `Hold`s
//!   money-only.
//! - **It defines no `Lot`.** [`Representation::Lot`] is catalog **policy**
//!   (D-E6-12), and a listing whose entry declares it is refused at creation: the lot
//!   is `mush-inventory`'s in E7 cut 2, with a different field list (10b §3.2, Q2D),
//!   and a second definition here would invert the intended edge.
//! - **It adds no settlement machinery.** Accepting a quote drafts an ordinary E4b2
//!   contract; `Deliverable::TitleTransfer` is E4's own; everything else is delivered
//!   by a registered provider through the economy's existing seams.
//! - **It moves no money itself.** Every escrow, payout and drip is an E4 act under an
//!   E4 authority, and no `@contract`, `@obligation`, `@asset`, `@vault` or `@org`
//!   command is ever dispatched from a handler here.
//!
//! ## What is where
//!
//! | file | what |
//! |---|---|
//! | `projection.rs` | **the gate** — [`Projection`], the argument-free fold, bucketed depth |
//! | `record.rs` | the five families, their caps, and the `TryFrom` wires that re-run them |
//! | `store.rs` | [`MarketStore`], the keys, the single write paths, the total [`load`] |
//! | `api.rs` | the catalog, vendors, listings, makers, and the per-viewer page |
//! | `trade.rs` | quotes, the buyer-as-issuer purchase, `+market/sell`, the settlement ear |
//! | `module.rs` | [`MarketModule`], the three wiring-frozen tables, [`MarketObserver`] |
//! | `commands.rs` | `+market` and `@market` |
//!
//! ## Wiring
//!
//! ```ignore
//! // AFTER mush-economy (it validates listings against real accounts, assets and
//! // vaults at boot) and BEFORE the rulesets, which register into its tables.
//! Box::new(mush_market::MarketModule::new()
//!     .with_catalog_sources(..)
//!     .with_availability(..)
//!     .with_delivery(..)),
//! ```
//!
//! and `mush_market::MarketObserver` into `EconomyModule::with_observers`, which is
//! how a settlement reaches the trade tape.

mod api;
mod commands;
mod ids;
mod module;
mod projection;
mod record;
mod store;
mod trade;

pub use api::{
    DEFAULT_LISTING_TTL_SECS, ListingSpec, MAX_LISTING_TTL_SECS, MarketError, MarketFilter,
    MarketPage, NO_SUCH_ENTRY, NO_SUCH_LISTING, NO_SUCH_QUOTE, NO_SUCH_SOURCE, NO_SUCH_VENDOR,
    PackLoad, account_instrument, budget_source, create_source, fund_from_faucet, halt_source,
    hidden_seller_orgs, is_hidden_org, is_staff, list_goods, load_pack, may_list_vendor,
    may_view_listing, principal_account, refuse_market, register_vendor, retire_vendor, visible,
    withdraw_listing,
};
pub use ids::{
    ListingId, MAX_MARKET_ID_BYTES, MarketId, MarketSourceId, QuoteId, VendorId, parse_u64,
};
pub use module::{
    AvailabilityProvider, CatalogSource, DeliveryOrder, DeliveryProvider, MarketModule,
    MarketObserver,
};
pub use projection::{
    DEPTH_BUCKETS, DEPTH_QUANTITY_STEP, DepthBucket, PROJECTION_VISIT, PROJECTION_WINDOW_SECS,
    Projection, is_public_seller,
};
pub use record::{
    AvailabilityOutcome, AvailabilityRule, CatalogEntry, Legality, Listing, MAX_CATALOG_CODE_BYTES,
    MAX_CATALOG_ENTRIES, MAX_CATALOG_ENTRIES_PER_PACK, MAX_ENTRY_NAME_BYTES, MAX_ENTRY_TAGS,
    MAX_LISTINGS, MAX_LISTINGS_PER_VENDOR, MAX_OPEN_QUOTES_PER_BUYER, MAX_QUANTITY, MAX_QUOTES,
    MAX_SOURCES, MAX_VENDORS, MarketSource, MarketVisibility, ObservedRevisions, Quote,
    RecordError, Representation, Settlement, Supply, Vendor, VendorStatus, check_entry,
    declared_hash,
};
pub use store::{
    MARKET_PAGE, MARKET_PAGE_MAX, MARKET_PAGE_VISIT, MAX_TRADES_PER_PAIR, META_KEY, MarketStore,
    NS, catalog_key, listing_key, load, quote_key, source_key, vendor_key,
};
pub use trade::{
    DEAL_WINDOW_SECS, MAX_LAPSED_SWEEP_PER_CALL, QUOTE_TTL_LIVE, QUOTE_TTL_REMOTE, QuoteRequest,
    QuoteWindow, STALE_QUOTE, accept, artefact_stamp, is_current, quote, sell,
};

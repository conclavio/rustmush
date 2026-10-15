//! `mush-market` — the **neutral market** (E6c, `docs/design/08f` §5).
//!
//! Bounded catalogs and content packs, listings, buyer-as-issuer quotes with declared
//! expiry clocks, vendors from stock / market source / procurement, NPC market makers
//! with explicit budgets, and — first, because everything else could leak into it —
//! **the safe aggregate public projection**.
//!
//! ## Game-neutral, and that is a dependency claim
//!
//! `mush-market → {core, org, economy, inventory}` plus
//! serde/serde_json/smol_str/tracing, and **nothing else**: no `mush-party`, no
//! `mush-canon`, no ruleset edge. Nothing in this crate names a run, a fence, nuyen or
//! a runner (answer 20 — it must serve WoD and TacSim too). The economy never depends
//! on the market (doc 08 D11, doc 10 §14), so `cargo tree -p mush-market` is the test
//! of both halves, and `tests/e7r2.rs` is its checked-in half.
//!
//! **`mush-inventory` is E7 residue pass 2's one new edge** (10b §15.1 item 3, §15.5
//! (a′)), and it runs in the direction 10b §3.2 always intended. It exists so a
//! `Supply::Lot` listing can reserve, consume and release the quantity it advertises
//! through cut 2's **own** exported verbs on the **delegated** path — the composition
//! root names `"market"` in `InventoryModule::with_quantity_delegates`, exactly as it
//! has named `"industry"` since cut 3 — rather than by defining a second quantity
//! authority here. No `InventoryVerb` is added, no `ENGINE_OWNED_RESERVATIONS` entry is
//! registered, and `mint_origin` is called nowhere in this crate.
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
//! through the existence-hiding renderer. `crate::projection`'s module header
//! states the five parts in full.
//!
//! ## What this crate does **not** do
//!
//! - **A quote reserves nothing.** A quote is a price with an expiry; a listing's
//!   quantity decrements at contract **activation** and nowhere else (OPEN 4). This
//!   crate still defines **no reservation family** and no new `Hold` field — 08i §4
//!   rule 1's exemption covers neither — and doc 10 T4 keeps E4 `Hold`s money-only.
//!   *E7r2 P1 narrows the sentence and does not delete it:* a `Supply::Lot` listing
//!   **does** reserve, on `mush-inventory`'s own `ReservationBook` through the
//!   delegated families, because it advertises finite matter another owner is the
//!   truth for rather than a counter of this crate's own. The divergence is recorded
//!   in 08f §5.6 and in the [`Supply::Lot`] doc comment.
//! - **It still defines no `Lot`.** [`Representation::Lot`] is catalog **policy**
//!   (D-E6-12) and the record is `mush-inventory`'s, with a different field list (10b
//!   §3.2, Q2D). What E7r2 P1 lifted is the *refusal*, not the ownership: this crate
//!   holds an opaque [`LotEndpoint`] and [`LotCommodity`] handle and reaches every
//!   quantity through cut 2's exported verbs, so there is still exactly one definition
//!   of a lot in the tree and exactly one place its arithmetic lives.
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
//! | `boot.rs` | the boot reconcile against the economy's healed contracts, and the bounded sweep |
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

// WS01-S12: the panic-shaped restriction lints this crate measures zero of,
// held at zero. `crates/mush-contract-tests/tests/restriction_lints.rs` carries
// the measurement this set was cut from — including the counts of the lints
// NOT named here — and fails if this attribute loses one of them.
#![deny(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable,
    clippy::panic_in_result_fn
)]

mod api;
mod boot;
mod commands;
mod ids;
pub mod interface;
mod lot;
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
    may_view_listing, org_is_closed, org_may_trade, principal_account, refuse_market,
    register_vendor, retire_vendor, visible, withdraw_listing,
};
pub use boot::{
    MAX_BOOT_DELIVERY_PAGES, MAX_LOT_RECONCILED_PER_FIRE, MAX_RECONCILED_QUOTES, MAX_SWEPT_PER_FIRE,
};
pub use ids::{
    ListingId, MAX_MARKET_ID_BYTES, MarketId, MarketSourceId, QuoteId, VendorId, parse_u64,
};
pub use lot::{
    ACTION_LOT, HoldStanding, LOT_REFUSED, LotError, LotOp, MAX_LOT_ENDPOINT_CHOICES,
    OUTCOME_ID as LOT_OUTCOME_ID, action_ref as lot_action_ref, hold_ttl_real_secs, listing_holder,
    listing_holder_text, listing_reservation,
};
pub use module::{
    AvailabilityProvider, CatalogSource, DeliveryOrder, DeliveryProvider, MarketModule,
    MarketObserver, SWEEP_PERIOD_SECS, SWEEP_TAG, lot_supply,
};
pub use projection::{
    DEPTH_BUCKETS, DEPTH_QUANTITY_STEP, DepthBucket, PROJECTION_VISIT, PROJECTION_WINDOW_SECS,
    Projection, is_public_seller,
};
pub use record::{
    AvailabilityOutcome, AvailabilityRule, CatalogEntry, Legality, Listing, LotCommodity,
    LotEndpoint, MAX_CATALOG_CODE_BYTES, MAX_CATALOG_ENTRIES, MAX_CATALOG_ENTRIES_PER_PACK,
    MAX_ENTRY_NAME_BYTES, MAX_ENTRY_TAGS, MAX_LISTINGS, MAX_LISTINGS_PER_VENDOR,
    MAX_OPEN_QUOTES_PER_BUYER, MAX_PACK_BYTES, MAX_QUANTITY, MAX_QUOTES, MAX_SOURCES,
    MAX_TRADE_ROWS, MAX_VENDORS, MarketSource, MarketVisibility, ObservedRevisions, Quote,
    RecordError, Representation, Settlement, Supply, Vendor, VendorStatus, check_entry,
    declared_hash,
};
pub use store::{
    MARKET_PAGE, MARKET_PAGE_MAX, MARKET_PAGE_VISIT, MAX_TRADES_PER_PAIR, META_KEY, MarketStore,
    NS, catalog_key, listing_key, load, owner, quote_key, source_key, vendor_key,
};
pub use trade::{
    DEAL_WINDOW_SECS, MAX_LAPSED_SWEEP_PER_CALL, MAX_OPEN_SELLS_PER_VENDOR, QUOTE_TTL_LIVE,
    QUOTE_TTL_REMOTE, QuoteRequest, QuoteWindow, STALE_QUOTE, accept, accept_into, artefact_stamp,
    is_current, quote, sell,
};

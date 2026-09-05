//! **The market's boot reconcile and its sweep** (E6c independent verify fold, R1–R4,
//! R6, R10, M4).
//!
//! E6c shipped with neither. `MarketModule::on_world_loaded` called `store::load` and
//! stopped, and the module registered no tick at all, so:
//!
//! - **a settlement healed by economy boot never reached the market.** The economy's
//!   own `boot::finished_transition` finishes a terminal transition whose doc write was
//!   lost by reading the contract's deterministic idempotency key, and persists the
//!   result with a **raw document write** — no `EconomicEvent::ContractChanged`. The one
//!   re-announcement path, `sweep::republish_settlements`, filters on
//!   `consent.visibility.is_public()` and a market purchase is `Parties`. So after a
//!   crash between a purchase's settle post and its doc write the contract came back
//!   `Settled`, the observer never ran, the goods were never handed over though the
//!   money had moved, the trade tape lost the row permanently, and the quote —
//!   `contract: Some`, `settled: None` — counted as open for ever against its buyer's
//!   eight (R1). The cancellation direction is the same defect: the stock and the
//!   maker's window were never given back (R2);
//! - **nothing ever deleted a lapsed listing or an aged tape row.** A listing left
//!   `live` for ever, holding one of its vendor's 64 slots and rendering as buyable; 64
//!   traded pairs' rows filled `MAX_QUOTES` 2 048 and `+market/quote` then answered
//!   "The market is busy; try later." to every buyer for ever, with no new trade
//!   possible to evict anything (R3, R4).
//!
//! Both are one pass, and the boot runs it once before the first tick: the reconcile is
//! the sweep plus the contract-state questions only a boot can be sure the economy has
//! already healed. `MarketModule` boots **after** `mush-economy`, so `EconomyStore` is
//! hydrated and healed by the time anything here asks it a question.
//!
//! **Bounded and panic-free.** Every walk takes a per-fire budget, there is no `?`,
//! `unwrap`, `expect` or panicking index anywhere in the file, and a delivery that
//! cannot run pages the economy's own level-2 ladder rather than parking on staff.

use mush_core::Ctx;
use mush_economy::{ContractState, EconomyStore};

use crate::ids::{ListingId, QuoteId};
use crate::record::Quote;
use crate::store::{MarketStore, del_listing, del_quote};

/// The most quotes one boot reconcile examines.
///
/// `MAX_QUOTES` is 2 048 and the pass is one contract lookup per quote, so the whole
/// population fits comfortably in one boot; the budget is here so a store that somehow
/// grew past its cap still finishes.
pub const MAX_RECONCILED_QUOTES: usize = 2_048;

/// The most records one **sweep fire** deletes, listings and quotes together — E6a's
/// `del_run` shape: a sweep that tried to clear a whole backlog in one fire would be an
/// unbounded write batch on the single-writer world task.
pub const MAX_SWEPT_PER_FIRE: usize = 64;

/// The most paid-and-undelivered purchases one boot pages about. The *deliveries* are
/// not capped — every one of them is attempted — only the pages, so a mass loss is one
/// readable alert list rather than four thousand.
pub const MAX_BOOT_DELIVERY_PAGES: usize = 8;

/// The whole boot pass: the sweep, then the contract reconcile.
pub(crate) fn reconcile(ctx: &mut Ctx<'_>) {
    sweep(ctx, usize::MAX);
    reconcile_contracts(ctx);
}

/// The `Coarse` sweep tick.
pub(crate) fn sweep_tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>) {
    sweep(ctx, MAX_SWEPT_PER_FIRE);
}

/// **Delete what has closed itself** (R3, R4).
///
/// Three families, all keyed on a declared deadline and nothing else:
///
/// - a **listing** whose `RealUtc` deadline has passed. It is the projection's own
///   `lapsed` test, so the page and the projection agree about what is gone;
/// - an **open quote** past its own expiry that names no contract. A quote naming one
///   is the contract's to close, and `on_contract_changed` does it;
/// - a **trade-tape row** whose settlement is older than the window the projection
///   folds. `MAX_TRADES_PER_PAIR` then becomes the second bound rather than the only
///   one, which is what stops a full tape refusing every new quote.
///
/// A deadline on any clock but `RealUtc` is left to whoever owns that clock, exactly as
/// the projection leaves it.
fn sweep(ctx: &mut Ctx<'_>, budget: usize) -> usize {
    let now = ctx.w.now();
    let horizon = now.saturating_sub(crate::projection::PROJECTION_WINDOW_SECS);
    let (listings, quotes) = {
        let Some(store) = ctx.w.module_state::<MarketStore>() else {
            return 0;
        };
        let listings: Vec<ListingId> = store
            .listings
            .values()
            .filter(|l| crate::projection::lapsed(l.expires.at, l.expires.clock, now))
            .map(|l| l.id)
            .take(budget)
            .collect();
        let left = budget.saturating_sub(listings.len());
        let quotes: Vec<QuoteId> = store
            .quotes
            .values()
            .filter(|q| match q.settled {
                // A settled row is the tape: it goes when the window rolls past it.
                Some(s) => s.at < horizon,
                // An open offer is a price: it goes when it lapses, unless a contract
                // is standing on it.
                None => {
                    q.contract.is_none()
                        && crate::projection::lapsed(q.expires.at, q.expires.clock, now)
                }
            })
            .map(|q| q.id)
            .take(left)
            .collect();
        (listings, quotes)
    };
    let swept = listings.len().saturating_add(quotes.len());
    for id in listings {
        tracing::debug!(listing = id.0, "market: a lapsed listing was withdrawn");
        del_listing(ctx, id);
    }
    for id in quotes {
        tracing::debug!(quote = id.0, "market: a closed quote was forgotten");
        del_quote(ctx, id);
    }
    swept
}

/// **Ask the economy about every quote that named a contract** (R1, R2, M4).
///
/// The economy has already healed itself by the time this runs, so its answer is
/// authoritative and no guessing is needed:
///
/// | the contract says | what the market does |
/// |---|---|---|
/// | `Settled { transaction }`, row unstamped | stamp the row from the **transaction's own `posted_at`** and run the delivery once |
/// | `Cancelled` / `Expired` | give the stock back, give the maker's window its budget back, forget the quote |
/// | gone entirely | forget the quote — a handle on a contract the economy does not have is not a handle |
/// | anything else | leave it exactly as it is |
///
/// Every arm keys on the quote's own existence, so a reconcile that runs twice does
/// nothing the second time.
fn reconcile_contracts(ctx: &mut Ctx<'_>) {
    let pending: Vec<Quote> = ctx
        .w
        .module_state::<MarketStore>()
        .map(|s| {
            s.quotes()
                .filter(|q| q.contract.is_some())
                .take(MAX_RECONCILED_QUOTES)
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let mut paged = 0usize;
    for q in pending {
        let Some(contract) = q.contract else {
            continue;
        };
        let state = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().contract(contract).map(|c| c.state));
        match state {
            Some(ContractState::Settled { transaction }) if q.settled.is_none() => {
                tracing::info!(
                    quote = q.id.0,
                    contract = contract.0,
                    "market: a settlement was healed by the economy and never announced; stamping \
                     the trade row and delivering"
                );
                crate::trade::stamp_settlement(ctx, &q, transaction);
                if paged < MAX_BOOT_DELIVERY_PAGES {
                    paged = paged.saturating_add(1);
                    crate::module::deliver(ctx, &q, contract);
                } else {
                    tracing::error!(
                        quote = q.id.0,
                        contract = contract.0,
                        "market: past this boot's delivery-page budget; goods undelivered"
                    );
                }
            }
            Some(ContractState::Cancelled | ContractState::Expired) => {
                tracing::info!(
                    quote = q.id.0,
                    contract = contract.0,
                    "market: a cancellation was healed by the economy and never announced; \
                     returning the stock and the maker's window"
                );
                crate::trade::release_quote(ctx, &q);
            }
            None => {
                tracing::warn!(
                    quote = q.id.0,
                    contract = contract.0,
                    "market: a quote names a contract the economy does not have; forgetting it"
                );
                crate::trade::release_quote(ctx, &q);
            }
            Some(_) => {}
        }
    }
    retire_closed_orgs(ctx);
}

/// **A vendor whose organization reached `Closed` stops being a shop** (M4).
///
/// DL-06 made `Closed` terminal and `@org/status` its only write path, and nothing in
/// the market ever read `OrgStatus` — so a closed organization went on trading, and
/// B10's one staff verb for shutting a shop down had no effect on the market at all.
/// `Suspended` and `Dissolving` are refused live, at the quote and the accept, because
/// they are reversible; `Closed` is not, so its vendor is retired here and its listings
/// are withdrawn with it.
fn retire_closed_orgs(ctx: &mut Ctx<'_>) {
    let closed: Vec<crate::ids::VendorId> = ctx
        .w
        .module_state::<MarketStore>()
        .map(|s| {
            s.vendors()
                .filter(|v| v.is_trading())
                .map(|v| (v.id, v.org))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, org)| crate::api::org_is_closed(ctx.w, *org))
        .map(|(id, _)| id)
        .collect();
    for id in closed {
        tracing::info!(
            vendor = id.0,
            "market: the organization behind a vendor is closed; retiring the shop"
        );
        let _ = crate::api::retire_vendor(ctx, id);
    }
}

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

/// How many sweeps a kept lapsed hold-backed listing goes between two reads of its
/// hold's standing (doc 18 cut IC14 row 16): the sweep keeps such a listing until its
/// hold reads `Gone`, and on a roster without a releaser that read would otherwise run
/// for every kept listing on every sweep. A hold that goes `Gone` is therefore deleted
/// within this many sweeps (ten minutes at the 60 s sweep).
pub const HOLD_RECHECK_SWEEPS: u64 = 10;

/// The most lot listings one sweep fire reconciles (E7r2 P1).
///
/// Bounded on records **visited** (D-LBK-5), with its own budget so it can never
/// starve the lapse sweep above it.
pub const MAX_LOT_RECONCILED_PER_FIRE: usize = 32;

/// The most undrawn Draw quotes one sweep fire mirrors (doc 18 cut IC7 item 7).
///
/// Its own budget on records **mirrored**, so it can never starve the arms above it;
/// the walk that finds them is bounded by the store's own `MAX_QUOTES` cap. A full lap
/// over N undrawn Draws takes `ceil(N / 64)` fires (`draw_lap`, unit-tested). With the
/// cutoff's 600 s margin, a 60 s market sweep and a 30 s economy sweep, the
/// `market.lot.drawn` fact is true before a landed Draw's deadline while at most
/// `64 × 9 = 576` undrawn Draw quotes stand
/// (`crate::DRAW_SETTLE_MARGIN_SECS` states the arithmetic, including the economy's own
/// per-sweep budget); past that, `release_quote`'s no-relist guard and its page are the
/// backstop.
pub const MAX_DRAWS_MIRRORED_PER_FIRE: usize = 64;

/// The `Coarse` sweep tick — **three arms on one existing tick**, never a second tick.
pub(crate) fn sweep_tick(ctx: &mut Ctx<'_>, _tag: &str, _scene: Option<mush_core::SceneId>) {
    fold_healed_sales(ctx);
    sweep(ctx, MAX_SWEPT_PER_FIRE);
    reconcile_lot_holds(ctx, MAX_LOT_RECONCILED_PER_FIRE);
    reconcile_draws(ctx, MAX_DRAWS_MIRRORED_PER_FIRE);
}

/// ---- doc 18 cut IC14 row 9: a boot-healed settlement folds once ----
///
/// A sale into a bid whose settlement the economy healed at boot and never announced is
/// stamped by [`reconcile_contracts`], which cannot tell the proceeds policy: the game
/// layer's store boots after the built-ins and is not hydrated yet (lead ruling L4).
/// The ids wait in `MarketStore::healed_sales` (RAM) and are drained here, at the top of
/// the first sweep after boot, each told to the policy through the settlement ear's own
/// `settled_sale` exactly once — the list is taken whole, and a stamped quote is never
/// pushed again. A crash between the boot and this sweep loses the fold (recorded on the
/// field).
fn fold_healed_sales(ctx: &mut Ctx<'_>) {
    let healed = ctx
        .w
        .module_state_mut::<MarketStore>()
        .map(|s| std::mem::take(&mut s.healed_sales))
        .unwrap_or_default();
    for id in healed {
        let Some(q) = ctx
            .w
            .module_state::<MarketStore>()
            .and_then(|s| s.quote(id).cloned())
        else {
            continue;
        };
        let (Some(contract), Some(_)) = (q.contract, q.settled.as_ref()) else {
            continue;
        };
        tracing::info!(
            quote = id.0,
            contract = contract.0,
            "market: telling the proceeds policy of a sale the economy healed at boot"
        );
        crate::trade::settled_sale(ctx, &q, contract);
    }
}

/// ---- doc 18 cut IC7: the draw mirror (item 7) ----
///
/// A Draw writes its durable receipt (`market_lot_draws[contract]`) in its own bundle,
/// and a `FactProvider` reads only `&World`, so the receipt is mirrored onto the quote's
/// `drawn_at` here — the **only** writer of `drawn_at`, which makes the
/// `market.lot.drawn` fact behind the receipt by at most the rotation and never ahead
/// (the inventory mirror's rule).
///
/// It visits up to `budget` quotes that are an activated, unsettled Draw with a contract
/// and no `drawn_at`, round-robin from the store's RAM cursor (IC5's rotation idiom),
/// and reads each one's receipt: found → `drawn_at = now` (real) with the revision
/// bumped; absent → left alone; unreadable → left alone, one warning per fire. It is a
/// third arm on the existing `SWEEP_TAG` tick and it stages nothing authoritative (a
/// `reg.tick` runs as GOD).
///
/// It is also **where an undrawn Draw quote whose contract lapsed or was cancelled is
/// released**: the settlement ear runs on the economy's context, where the owner-scoped
/// receipt read is refused, so it leaves such a quote alone
/// (`trade::awaits_receipt_read`) and this arm, on the market's own context, hands it to
/// `release_quote` — which reads the receipt and restores the listing only when nothing
/// moved. The listing therefore comes back within one lap of the expiry rather than in
/// the economy's own fire.
///
/// **Doc 18 cut IC10: the same lap releases a lapsed sale into a bid.** An activated,
/// unsettled `PaidKind::Deliver` quote with a contract joins the candidate set (same
/// budget, same cursor), and for it the arm does **only** the terminal check — its
/// contract `Cancelled`, `Expired` or gone → `release_quote`, which reads the sale's
/// `inventory.transferred` marker on this, the market's own context, and restores the
/// bid only when nothing moved. It never mirrors anything: the sale's fact is
/// inventory's own.
fn reconcile_draws(ctx: &mut Ctx<'_>, budget: usize) {
    let candidates: Vec<Quote> = {
        let Some(store) = ctx.w.module_state::<MarketStore>() else {
            return;
        };
        // The undrawn Draw ids and (cut IC10) the in-flight sale ids, ascending (the
        // map's order); `MAX_QUOTES` bounds the walk.
        let undrawn: Vec<QuoteId> = store
            .quotes
            .values()
            .filter(|q| {
                q.activated
                    && q.settled.is_none()
                    && q.drawn_at.is_none()
                    && q.contract.is_some()
                    && q.lot.as_ref().is_some_and(|l| {
                        matches!(
                            l.kind,
                            crate::record::PaidKind::Draw | crate::record::PaidKind::Deliver
                        )
                    })
            })
            .map(|q| q.id)
            .collect();
        draw_lap(&undrawn, store.draw_cursor, budget)
            .into_iter()
            .filter_map(|id| store.quotes.get(&id).cloned())
            .collect()
    };
    let mut warned = false;
    let mut last: Option<QuoteId> = None;
    for q in candidates {
        last = Some(q.id);
        let Some(contract) = q.contract else {
            continue;
        };
        // A lapsed or cancelled deal is released here, where the receipt read is the
        // market's own (see the doc comment); nothing is mirrored onto a closed deal.
        let state = ctx
            .w
            .module_state::<EconomyStore>()
            .and_then(|s| s.snapshot().contract(contract).map(|c| c.state));
        // Doc 18 cut IC10: a sale into a bid gets the terminal check only.
        if q.lot
            .as_ref()
            .is_some_and(|l| l.kind == crate::record::PaidKind::Deliver)
        {
            if matches!(
                state,
                None | Some(ContractState::Cancelled | ContractState::Expired)
            ) {
                crate::trade::release_quote(ctx, &q);
            }
            continue;
        }
        if matches!(
            state,
            Some(ContractState::Cancelled | ContractState::Expired)
        ) {
            crate::trade::release_quote(ctx, &q);
            continue;
        }
        match crate::lot::draw_receipt(ctx, contract) {
            Ok(Some(_)) => {
                let now = ctx.w.now();
                crate::store::put_quote(
                    ctx,
                    Quote {
                        drawn_at: Some(now),
                        revision: q.revision.saturating_add(1),
                        ..q
                    },
                );
            }
            Ok(None) => {}
            Err(()) => {
                if !warned {
                    warned = true;
                    tracing::warn!(
                        quote = q.id.0,
                        contract = contract.0,
                        "market: a Draw's receipt could not be read; the mirror will try again"
                    );
                }
            }
        }
    }
    // The cursor is RAM only and never persisted. It is the last quote VISITED in every
    // case — after a wrap, the last of the wrapped prefix — so the next fire resumes
    // exactly where this one stopped and a full lap over N undrawn Draws takes
    // `ceil(N / budget)` fires, never one more (resetting to `None` after a wrap would
    // re-walk the prefix just visited and add a wasted fire to every lap, breaking the
    // margin's arithmetic on `crate::DRAW_SETTLE_MARGIN_SECS`). Nothing visited: back to
    // the start.
    if let Some(store) = ctx.w.module_state_mut::<MarketStore>() {
        store.draw_cursor = last;
    }
}

/// One fire's lap over the undrawn Draw ids (`undrawn`, ascending), resuming after
/// `after` (doc 18 cut IC7 item 7): the ids past the cursor first, then — on a short lap
/// with a cursor set — the ids from the lowest up, never one visited twice in a fire and
/// at most `budget` in all. With the caller storing the last id returned as the next
/// cursor, a full lap over N ids takes exactly `ceil(N / budget)` fires, which is the
/// arithmetic `crate::DRAW_SETTLE_MARGIN_SECS` rests on (pinned by the unit test below).
fn draw_lap(undrawn: &[QuoteId], after: Option<QuoteId>, budget: usize) -> Vec<QuoteId> {
    let split = after.map_or(0, |c| undrawn.partition_point(|id| *id <= c));
    let mut picked: Vec<QuoteId> = undrawn
        .get(split..)
        .unwrap_or_default()
        .iter()
        .copied()
        .take(budget)
        .collect();
    if picked.len() < budget && after.is_some() {
        let left = budget.saturating_sub(picked.len());
        picked.extend(
            undrawn
                .get(..split)
                .unwrap_or_default()
                .iter()
                .copied()
                .take(left),
        );
    }
    picked
}

/// ---- E7r2: P1 — the arm OPEN 2's ordering owes ----
///
/// `@market/vendor stock` writes the listing document and *then* stages the one
/// authoritative submission that places its hold, because a command cannot observe its
/// own submission. A crash in between therefore leaves a **live `Supply::Lot` listing
/// whose reservation is not `Held`** — a listing a buyer could pay for and whose matter
/// leg would then refuse.
///
/// This arm withdraws exactly those, and **pages per listing** rather than per fire, so
/// one stuck listing stops only itself (08c §4.6: stop only the subject, with the
/// recommended command filled in). It is a NEW ARM on the market's **existing**
/// `SWEEP_TAG` tick — no second tick, no clock, no schedule source — and it stages **no
/// authoritative submission at all**: a `reg.tick` runs as GOD and two GOD-tick
/// submissions in one drain void both. The withdrawal is a plain document delete, and
/// the hold it could not find is the one thing it never has to release.
///
/// A reboot re-running it is a no-op past the applied point: a listing whose hold IS
/// live is left exactly as it is, and a listing already withdrawn is not in `live`.
fn reconcile_lot_holds(ctx: &mut Ctx<'_>, budget: usize) {
    let Some(families) = crate::module::lot_supply(ctx.w) else {
        return;
    };
    let candidates: Vec<(crate::ids::ListingId, mush_inventory::EndpointRef, String)> = ctx
        .w
        .module_state::<MarketStore>()
        .map(|store| {
            store
                .lot_listings()
                .take(budget)
                .filter_map(|listing| {
                    listing.source.lot().map(|(endpoint, commodity, _)| {
                        (listing.id, endpoint, commodity.as_str().to_owned())
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    for (id, endpoint, commodity) in candidates {
        let Ok(commodity) = mush_core::CanonicalId::new(&commodity) else {
            continue;
        };
        let Ok(reservation) = crate::lot::listing_reservation(id, endpoint, &commodity) else {
            continue;
        };
        // The reservation index is written in the same bundle as the hold, so a live
        // hold is a point lookup rather than a walk (cut 2's fold B10).
        let held = ctx
            .action_state(&families.reservations, &reservation)
            .ok()
            .flatten()
            .is_some();
        if held {
            continue;
        }
        tracing::error!(
            listing = id.0,
            endpoint = endpoint.vault().0,
            "market: a lot listing names a reservation nothing holds; withdrawing it"
        );
        mush_economy::open_escalation(
            ctx,
            mush_economy::EscalationKind::SettlementRefused,
            mush_economy::EscalationSubject::MarketListing(id.0),
            format!(
                "market listing #{id} advertised inventory stock and its reservation was never \
                 placed. The listing has been withdrawn; nothing was sold and no quantity moved."
            ),
            vec![
                format!("@inv/show {}", endpoint.vault().0),
                format!(
                    "@market/vendor stock <vendor>=<ruleset>:<code> price=<n> endpoint={}",
                    endpoint.vault().0
                ),
            ],
        );
        del_listing(ctx, id);
    }
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
///
/// **A lapsed hold-backed listing waits for its hold** (doc 18 cut IC8, lead ruling L7;
/// D-18-26, Q-18-31): this GOD sweep deletes a lapsed `Supply::Lot` listing only once
/// [`crate::lot::hold_standing`] reads `Gone`. Live, Lapsed, Unreadable — anything else
/// — keeps the document (it is already unsellable, because it has lapsed), and a kept
/// listing is filtered out **before** the budget is taken, so it never spends
/// `MAX_SWEPT_PER_FIRE` and cannot starve the deletion of anything else. The release is
/// the holder's own authoritative submission, never a GOD tick's
/// (`mush-campaign/src/tick.rs:28-37`): the TacSim layer's port tick makes it as the
/// holder through [`crate::release_lapsed_hold`]; on a roster without a releaser the
/// seller withdraws the listing by hand, as cut IC0 recorded. Before IC8 this sweep
/// deleted the document and stranded the hold.
///
/// **An exhausted standing bid is never deleted by a lapse** (doc 18 cut IC10; D-18-27
/// "no restock"): a lapsed `Supply::Wanted` at quantity 0 is kept, filtered out before
/// the budget like the kept hold-backed listing, so a taken intake is never re-opened.
/// A lapsed bid with intake left is deleted as before, and the stock clerk re-posts it.
fn sweep(ctx: &mut Ctx<'_>, budget: usize) -> usize {
    let now = ctx.w.now();
    let horizon = now.saturating_sub(crate::projection::PROJECTION_WINDOW_SECS);
    // The lapsed listings, each with whether it is hold-backed, collected under the
    // store borrow (bounded by the store's own `MAX_LISTINGS`); the hold reads need
    // `ctx`, so they are filtered below.
    let lapsed: Vec<(ListingId, bool)> = {
        let Some(store) = ctx.w.module_state::<MarketStore>() else {
            return 0;
        };
        store
            .listings
            .values()
            .filter(|l| crate::projection::lapsed(l.expires.at, l.expires.clock, now))
            .filter(|l| !(l.source.is_wanted() && l.quantity == 0))
            .map(|l| {
                let held = matches!(
                    store.lot_matter(l),
                    Some(crate::record::LotMatter::Hold { .. })
                );
                (l.id, held)
            })
            .collect()
    };
    // Doc 18 cut IC14 row 16: a per-listing backoff on the hold read. This sweep's
    // number, and the kept lapsed hold-backed set the backoff map is retained to.
    let sweep_no = ctx.w.module_state_mut::<MarketStore>().map_or(0, |s| {
        s.sweeps = s.sweeps.saturating_add(1);
        s.sweeps
    });
    let held_now: std::collections::BTreeSet<ListingId> = lapsed
        .iter()
        .filter(|(_, held)| *held)
        .map(|(id, _)| *id)
        .collect();
    let mut read_now: Vec<ListingId> = Vec::new();
    let listings: Vec<ListingId> = lapsed
        .into_iter()
        .filter(|(id, held)| {
            if !held {
                return true;
            }
            // Read within the last `HOLD_RECHECK_SWEEPS` sweeps: kept without a read.
            let last = ctx
                .w
                .module_state::<MarketStore>()
                .and_then(|s| s.hold_checked.get(id).copied());
            // A release staged within the window is read every sweep, so its landing
            // deletes the listing on the next sweep.
            let releasing = ctx
                .w
                .module_state::<MarketStore>()
                .and_then(|s| s.release_staged.get(id).copied())
                .is_some_and(|at| sweep_no.saturating_sub(at) <= HOLD_RECHECK_SWEEPS);
            if !releasing
                && last.is_some_and(|at| sweep_no.saturating_sub(at) < HOLD_RECHECK_SWEEPS)
            {
                return false;
            }
            read_now.push(*id);
            let standing = ctx
                .w
                .module_state::<MarketStore>()
                .and_then(|s| s.listing(*id))
                .and_then(|l| crate::lot::hold_standing(ctx, l));
            let gone = standing == Some(crate::lot::HoldStanding::Gone);
            if !gone {
                tracing::debug!(
                    listing = id.0,
                    ?standing,
                    "market: a lapsed hold-backed listing is kept until its hold is released"
                );
            }
            gone
        })
        .map(|(id, _)| id)
        .take(budget)
        .collect();
    if let Some(store) = ctx.w.module_state_mut::<MarketStore>() {
        store.hold_reads = store
            .hold_reads
            .saturating_add(u64::try_from(read_now.len()).unwrap_or(u64::MAX));
        for id in read_now {
            store.hold_checked.insert(id, sweep_no);
        }
        store.hold_checked.retain(|id, _| held_now.contains(id));
        store.release_staged.retain(|id, _| held_now.contains(id));
    }
    let quotes: Vec<QuoteId> = {
        let Some(store) = ctx.w.module_state::<MarketStore>() else {
            return 0;
        };
        let left = budget.saturating_sub(listings.len());
        store
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
            .collect()
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
                // Doc 18 cut IC14 row 9: a sale into a bid is folded into the game's
                // books on the first sweep (`fold_healed_sales`), once.
                if q.lot
                    .as_ref()
                    .is_some_and(|l| l.kind == crate::record::PaidKind::Deliver)
                    && let Some(store) = ctx.w.module_state_mut::<MarketStore>()
                {
                    store.healed_sales.push(q.id);
                }
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

#[cfg(test)]
mod tests {
    use super::draw_lap;
    use crate::ids::QuoteId;

    /// Drives `draw_lap` the way `reconcile_draws` does (the cursor is the last id
    /// visited) and returns the largest gap, in fires, between two visits of any id.
    fn widest_gap(n: u64, budget: usize, fires: usize) -> usize {
        let ids: Vec<QuoteId> = (1..=n).map(QuoteId).collect();
        let mut cursor: Option<QuoteId> = None;
        let mut seen_at: std::collections::BTreeMap<QuoteId, usize> = Default::default();
        let mut widest = 0;
        for fire in 0..fires {
            let lap = draw_lap(&ids, cursor, budget);
            assert!(lap.len() <= budget, "a fire visits at most its budget");
            let mut unique = lap.clone();
            unique.sort();
            unique.dedup();
            assert_eq!(
                unique.len(),
                lap.len(),
                "no id is visited twice in one fire"
            );
            for id in &lap {
                if let Some(prev) = seen_at.insert(*id, fire) {
                    widest = widest.max(fire - prev);
                }
            }
            cursor = lap.last().copied();
        }
        assert_eq!(seen_at.len() as u64, n, "every id is visited");
        widest
    }

    /// Doc 18 cut IC7 item 7 (reviewer R1): a full lap over N undrawn Draws takes
    /// `ceil(N / 64)` fires and never one more — at N = 576 = 64 × 9 every quote is
    /// re-visited within 9 fires (540 s), inside the 600 s cutoff margin with the 30 s
    /// economy sweep after it. A cursor reset after a wrap made it 10.
    #[test]
    fn ic7_a_draw_mirror_lap_is_ceil_n_over_the_budget_fires() {
        assert_eq!(widest_gap(576, 64, 40), 9);
        assert_eq!(widest_gap(512, 64, 40), 8);
        assert_eq!(widest_gap(100, 64, 40), 2);
        assert_eq!(widest_gap(64, 64, 10), 1);
        assert_eq!(widest_gap(10, 64, 10), 1);
        assert!(draw_lap(&[], Some(QuoteId(7)), 64).is_empty());
        assert!(draw_lap(&[], None, 64).is_empty());
    }
}

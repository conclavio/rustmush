//! **Buyer-as-issuer quotes and the purchase contract** (E6c, B6; §5.2, DL-11/G8,
//! D-E6-13, answer 11).
//!
//! ## The flow, and which authority each step uses
//!
//! | step | verb | authority |
//! |---|---|---|
//! | quote | `+market/quote <listing>[=<qty>]` | none — a quote is a **price**, and it reserves nothing |
//! | draft + offer | `+market/accept <quote>` | the **buyer is the issuer** (E4b2's only-the-issuer-offers rule): `offer_contract` runs with the buyer as actor |
//! | escrow | (the same call) | the buyer's **own** `accept_contract`, which places their consideration hold under their own account's debit authority |
//! | vendor accept | (the same call) | `accept_contract_as` under E6b's wiring-frozen `with_party_caps` table — which grants **no money authority whatever**: the escrow still posts under the vendor account's own debit authority and still refuses on `InsufficientAvailable`, which is exactly why "a maker that runs out stops quoting" needs no code (M19, D-E6-15) |
//! | activate | (the same call) | the last acceptance activates; **this is where `quantity` decrements** — the single serialization point (OPEN 4) |
//! | settle | the economy's own sweep/observer | `ContractAuth` |
//! | deliver | the wiring-frozen [`DeliveryProvider`](crate::DeliveryProvider) table | the provider's own, at settlement |
//!
//! **E6c adds no settlement machinery at all**: `TitleTransfer` is an ordinary E4b2
//! deliverable the settlement already executes, and everything else is delivered by a
//! registered provider through the economy's existing seams.
//!
//! ## What a quote does and does not do (OPEN 4)
//!
//! It records a price, a quantity, an expiry **on a declared clock**, every declared
//! revision it was priced against, and the recorded availability outcome when the
//! entry demanded one. It **reserves nothing**: 08i §4 rule 1's exemption explicitly
//! does not cover reservations or mechanical offers, and doc 10 T4 keeps E4 `Hold`s
//! money-only, so E6c ships no reservation family and no new `Hold` field. The
//! listing's `quantity` decrements at contract **activation** and nowhere else, so a
//! quote against a listing that sold out first refuses at `+market/accept` with the
//! same deterministic refusal a stale quote gets, and nothing is charged.

use mush_core::{Ctx, ObjId};
use mush_economy::{
    AcceptOutcome, Amount, ConsiderationSpec, ContractId, ContractRoles, ContractSpec,
    ContractState, Deliverable, EconomicClock, EconomicDeadline, EconomicVisibility, EconomyStore,
    EvidenceKind, Expiry, InstrumentId, OutcomeTerms, PayoutShare, Share, TypedCondition,
};
use mush_org::PrincipalId;
use smol_str::SmolStr;

use crate::api::{
    MarketError, NO_SUCH_LISTING, NO_SUCH_QUOTE, NO_SUCH_VENDOR, may_view_listing,
    principal_account,
};
use crate::ids::{ListingId, QuoteId};
use crate::module::{CapacityReadError, CapacityRefusal};
use crate::record::{
    AvailabilityRule, Listing, LotCommodity, LotEndpoint, LotMatter, MAX_OPEN_QUOTES_PER_BUYER,
    MAX_QUOTES, MAX_TRADE_ROWS, ObservedRevisions, PaidKind, PaidLot, Quote, Representation,
    Settlement, Supply, Vendor, declared_hash,
};
// Doc 18 cut IC10's lot sell arm (`sell_into_bid`, `restage_deliver`) lives below the
// object arm (`sell`), beside the settlement ear it feeds.
use crate::store::{MarketStore, del_quote, persist_meta, put_listing, put_quote};

/// A quote's expiry for an in-person session, in seconds (08f §9, `QUOTE_TTL_LIVE`).
pub const QUOTE_TTL_LIVE: i64 = 3_600;
/// A quote's expiry for an asynchronous offer, in seconds (`QUOTE_TTL_REMOTE`).
pub const QUOTE_TTL_REMOTE: i64 = 24 * 3_600;
/// How many **lapsed** quotes one `+market/quote` clears as it counts its buyer's own
/// (E6b independent verify fold, D4: a lapsed quote must never wedge its holder).
pub const MAX_LAPSED_SWEEP_PER_CALL: usize = 32;

/// How long a purchase contract stands before its deadline lapses.
pub const DEAL_WINDOW_SECS: i64 = 24 * 3_600;

/// **How long a Draw sale's contract stands** (doc 18 cut IC7, L2), in **real** seconds.
///
/// A Draw sale settles only on proof its goods moved (the `market.lot.drawn` fact); one
/// whose Draw is refused at prepare never sees that proof and expires with a refund at
/// this window, with no page. Short enough that a refused Draw locks the buyer's cash
/// for at most an hour (an IC day at rate 24), long enough for IC11's twelve-fire
/// re-stage. Hold-backed and every other sale keep [`DEAL_WINDOW_SECS`].
pub const DRAW_WINDOW_SECS: i64 = 3_600;

/// **The Draw cutoff's margin** (doc 18 cut IC7, L2, F1), in **real** seconds: no Draw is
/// offered — and so none is prepared, the driver projecting immediately before it
/// prepares — in the last `DRAW_SETTLE_MARGIN_SECS` of the window (`PaidLot::draw_by` =
/// accept + 3,000 s).
///
/// Why it exists: readiness is evaluated before expiry, but `Before` is false after the
/// deadline, so a Draw landing in the window's last seconds would be mirrored too late,
/// the contract would expire and refund while the goods had moved, and the listing would
/// be restored over goods that left. The arithmetic: a Draw lands no later than
/// `draw_by` = deadline − 600 s; the market sweep's mirror arm visits every undrawn Draw
/// within `ceil(N / MAX_DRAWS_MIRRORED_PER_FIRE)` market sweeps of 60 s (its cursor
/// resumes at the last quote visited, so a lap is never one fire longer), and the
/// economy sweeps every 30 s; so the fact is true before the deadline while
/// `N ≤ 64 × 9 = 576` undrawn Draw quotes stand (at least 72 buyers at the per-buyer cap
/// of 8): mirrored within 9 × 60 = 540 s, settled by the next economy sweep, 570 s in
/// all. The economy's own budget (`MAX_CONTRACTS_PER_SWEEP`, 64 contracts per 30 s
/// fire, `mush-economy/src/sweep.rs`) counts only contracts with a step due — ready to
/// settle or lapsed, never an `Active` contract still waiting — and it clears twice the
/// mirror's 64 per 60 s, so the bound holds while the economy's other due work leaves
/// it room. Past either, `release_quote`'s no-relist guard and its page are the
/// backstop.
pub const DRAW_SETTLE_MARGIN_SECS: i64 = 600;

/// Doc 18 cut IC7 (lead note 2e): a governed endpoint takes goods only from its own
/// port's counters.
pub const PORT_ONLY_ENDPOINT: &str = "That endpoint takes goods only from its port's own counters, so the sale was not made. \
     Nothing was charged.";

/// Doc 18 cut IC7: the capacity pre-check could not read a pool it needs — refused,
/// never read as zero.
pub const CAPACITY_UNREADABLE: &str = "That endpoint's load cannot be read right now, so the sale was not made. Nothing was \
     charged.";

/// Doc 18 cut IC8 (lead ruling L14): `+market/quote` and `+market/accept` on a standing
/// bid (`Supply::Wanted`) — refused before any write. Selling into a bid is cut IC10's.
pub const WANTED_IS_A_BID: &str =
    "That listing is a bid: its house buys that good and sells none. Nothing was charged.";

/// Doc 18 cut IC7: a re-stage asked for after the Draw cutoff.
pub const DRAW_TOO_LATE: &str =
    "It is too late to ask for those goods again. The deal lapses and your payment comes back.";

/// The goods leg cannot be honoured: E6c's undeliverable line, byte for byte, now shared
/// by the delivery pre-check, the Draw's source and fact-namespace pre-check, and a
/// re-stage the action layer would not take.
const UNDELIVERABLE: &str = "That cannot be delivered to you right now, so the sale was not made. \
                             Nothing was charged.";

/// The most `+market/sell` offers one seller may have standing against one vendor
/// (independent verify fold, M9).
///
/// The per-buyer quote cap's shape, on the other side of the counter: the vendor
/// escrows first, so every standing offer pins some of its available balance for
/// [`DEAL_WINDOW_SECS`], and without a cap the ceiling was the economy's
/// `MAX_OPEN_CONTRACTS_PER_ISSUER` — sixteen escrows against one shop, at no cost to
/// the offerer.
pub const MAX_OPEN_SELLS_PER_VENDOR: usize = 4;

/// The **exact** words a quote that can no longer be honoured is refused with.
///
/// One string for every reason (D-E6-13, scenario 29): lapsed, a moved listing
/// revision, a bumped catalogue definition version, a moved declared artefact, a
/// halted maker, a retired vendor, and a listing that sold out first. A buyer who
/// watched the price and waited gets the same answer as one who never looked, and the
/// difference between "somebody else bought it" and "the shop re-priced" is not
/// readable off the refusal.
pub const STALE_QUOTE: &str = "That quote no longer stands — the terms moved or it lapsed. Ask for a new one; nothing was \
     charged.";

/// Whether a quote is still good **right now** (D-E6-13, scenario 29).
///
/// Deterministic and total: it compares declared stamps for **equality** and never
/// re-prices. Every input is declared — the catalogue version, the listing's own
/// revision, and a hash of the declared catalogue code — so no hidden fact can move
/// it, which is E6b's D1 fix carried forward.
#[must_use]
pub fn is_current(ctx: &Ctx<'_>, q: &Quote) -> bool {
    let now = ctx.w.now();
    if now >= q.expires.at {
        return false;
    }
    let Some(store) = ctx.w.module_state::<MarketStore>() else {
        return false;
    };
    let Some(listing) = store.listing(q.listing) else {
        return false;
    };
    if listing.revision != q.observed.listing || listing.quantity < q.quantity {
        return false;
    }
    // **The listing's own deadline** (independent verify fold, M3). `quote` checks it
    // and `is_current` did not, so the vendor's declared window was enforced when the
    // price was asked for and dropped at the point money moves — and a quote TTL runs
    // to 30 days against a listing floor of 60 seconds, with nothing sweeping a lapsed
    // listing out of the store. The projection already treats such a listing as gone,
    // so the market was selling goods it publicly said were not for sale.
    if crate::projection::lapsed(listing.expires.at, listing.expires.clock, now) {
        return false;
    }
    // **The hold's own deadline** (doc 18 cut IC0). A `Supply::Lot` listing is backed
    // by a `mush-inventory` reservation with a deadline of its own, on the world's IC
    // clock, and nothing here read it: a quote taken against a lapsed hold passed every
    // declared-stamp check above, the money leg came into force, and the matter leg
    // then refused inside the settling bundle — the buyer paid and held nothing. The
    // hold is read through the typed-state seam the composition delegated to this
    // crate (the sweep's own reconcile read), and **anything but a live hold refuses**:
    // lapsed, consumed, released, never placed, on another clock generation (K4a
    // refuses the comparison rather than expiring early, and so does the settle), or
    // unreadable from this context. Fail closed, because this is the point money moves.
    if crate::lot::hold_standing(ctx, listing)
        .is_some_and(|standing| standing != crate::lot::HoldStanding::Live)
    {
        return false;
    }
    let Some(entry) = store.entry_of(&q.catalog) else {
        return false;
    };
    if entry.definition_version != q.observed.definition_version
        || q.definition_version != entry.definition_version
    {
        return false;
    }
    if artefact_stamp(entry.representation, &q.catalog) != q.observed.artefact {
        return false;
    }
    // A retired vendor and a halted maker both stop honouring quotes; both are
    // declared facts about the seller, and neither is a probe.
    if store.vendor(listing.vendor).is_none_or(|v| !v.is_trading()) {
        return false;
    }
    if let Supply::Market { source } = listing.source {
        let Some(maker) = store.source(source) else {
            return false;
        };
        if maker.halted {
            return false;
        }
        // **A re-budget lapses the quotes it re-priced** (independent verify fold, M7).
        // `spread` is a live pricing input `price_for` reads at quote time; nothing
        // stamped it, so a staff correction of a mis-set spread left every outstanding
        // quote executing at the old price for the whole window. Re-pricing through the
        // listing invalidated a quote and re-pricing through the spread did not; now
        // both join the single `STALE_QUOTE` family.
        if q.observed.source != Some(maker.revision) {
            return false;
        }
    }
    true
}

/// The declared artefact stamp a quote binds to — a stable hash of the **declared**
/// catalogue code, and `None` when the entry conveys no titled artefact.
///
/// Deliberately **not** an `EconomicAsset.revision`: E6b's verify fold (D1) proved
/// that construct is an oracle, because every asset op bumps the revision and a
/// **covert** theft report is an asset op. A declared code is moved only by a
/// declared act.
#[must_use]
pub fn artefact_stamp(rep: Representation, catalog: &mush_economy::CatalogRef) -> Option<u64> {
    matches!(rep, Representation::Asset).then(|| declared_hash(catalog.code.as_str()))
}

/// Which declared clock a quote's expiry is on (answer 11, D-E6-13).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum QuoteWindow {
    /// An in-person session: `QUOTE_TTL_LIVE`, on `RealUtc`.
    Live,
    /// An asynchronous offer: `QUOTE_TTL_REMOTE`, on `RealUtc`.
    Remote,
}

/// Everything `+market/quote` was asked for.
#[derive(Clone, Debug)]
pub struct QuoteRequest {
    /// Which listing.
    pub listing: ListingId,
    /// How many units.
    pub quantity: u64,
    /// Which declared window.
    pub window: QuoteWindow,
    /// The seconds each window lasts, from configuration.
    pub ttl_live: i64,
    /// The seconds a remote window lasts, from configuration.
    pub ttl_remote: i64,
    /// How the asker is present (doc 18 §4.3, cut IC7): a typed verb is
    /// [`Presence::Here`](crate::Presence::Here).
    pub presence: crate::module::Presence,
}

/// **Issue a quote** (B6, B7, B8).
///
/// A maker's budgets are checked **here**, before the quote is issued — never at
/// accept (D-E6-15): a maker that has run out simply stops quoting, and there is no
/// half-committed state to unwind. An availability test, when the entry declares one,
/// is resolved by the wiring-frozen provider table and **recorded on the quote**,
/// binding thereafter; a failure refuses **without charging**, because nothing has
/// been charged.
///
/// # Errors
///
/// [`MarketError`]; render it with [`crate::refuse_market`].
pub fn quote(ctx: &mut Ctx<'_>, req: &QuoteRequest) -> Result<QuoteId, MarketError> {
    let buyer = PrincipalId::Object(ctx.actor);
    let Some(listing) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.listing(req.listing).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    };
    // The existence-hiding gate comes first, and it is the same answer a nonexistent
    // listing gets.
    if !may_view_listing(ctx.w, ctx.actor, &listing) {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    }
    // Doc 18 cut IC8 (lead ruling L14): a standing bid buys and sells nothing, so it is
    // never quoted — refused before any write (the lapsed-quote sweep below writes).
    if listing.source.is_wanted() {
        return Err(MarketError::bad(WANTED_IS_A_BID));
    }
    let Some(vendor) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor(listing.vendor).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    };
    // A retired vendor and an organization DL-06 has stopped both refuse the same way —
    // one `STALE_QUOTE`, one family (independent verify fold, M4).
    if !vendor.is_trading() || !crate::api::org_may_trade(ctx.w, vendor.org) {
        return Err(MarketError::bad(STALE_QUOTE));
    }
    // **The presence gate** (doc 18 §4.3, cut IC7): a governed venue says in its own
    // words why the asker cannot trade there, and no quote is written. No gate is the
    // pre-IC7 behaviour exactly.
    if let Some(gate) = crate::module::venue_gate(ctx.w) {
        gate.admit(ctx.w, ctx.actor, &listing.market, None, req.presence)
            .map_err(|r| MarketError::bad(r.line))?;
    }
    let Some(entry) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&listing.catalog).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    };
    let quantity = req.quantity.max(1);
    if quantity > listing.quantity {
        return Err(MarketError::bad(STALE_QUOTE));
    }
    if ctx.w.now() >= listing.expires.at {
        return Err(MarketError::bad(STALE_QUOTE));
    }
    // **The hold behind a lot listing** (doc 18 cut IC0): a hold that has lapsed or
    // gone refuses the quote in the same words a lapsed listing does. A hold this
    // context cannot read (no lot supply composed, or a detached typed-state reader)
    // is not a refusal here — a quote reserves nothing, and `is_current` makes the
    // same read, fail-closed, at the point money moves.
    if crate::lot::hold_standing(ctx, &listing).is_some_and(|standing| {
        matches!(
            standing,
            crate::lot::HoldStanding::Lapsed | crate::lot::HoldStanding::Gone
        )
    }) {
        return Err(MarketError::bad(STALE_QUOTE));
    }
    let unit = price_for(&listing, ctx.w.module_state::<MarketStore>());
    let total = Amount::from_minor(
        unit.minor()
            .saturating_mul(i64::try_from(quantity).unwrap_or(i64::MAX)),
    );
    // ---- B8: every budget, before the quote is issued ---------------------
    let mut observed_source: Option<u64> = None;
    if let Supply::Market { source } = listing.source {
        let Some(maker) = ctx
            .w
            .module_state::<MarketStore>()
            .and_then(|s| s.source(source).cloned())
        else {
            return Err(MarketError::NoStanding(NO_SUCH_LISTING));
        };
        let now = ctx.w.now();
        let maker = maker.rolled(now);
        observed_source = Some(maker.revision);
        let loss = booked_loss(&entry, unit, quantity);
        if let Some(why) = maker.refuses(total, loss) {
            return Err(MarketError::bad(maker_refusal(why)));
        }
        // A maker that cannot pay for its own side is out of cash, and cash is the
        // fourth budget. It is asked the same way the vendor's is: of the account.
        if !can_cover_inventory(ctx, &maker) {
            return Err(MarketError::bad(maker_refusal("cash")));
        }
    }
    // ---- the per-buyer cap, sweeping the lapsed ones it passes ------------
    let mine: Vec<QuoteId> = ctx
        .w
        .module_state::<MarketStore>()
        .map(|s| {
            s.quotes_of_buyer(&buyer)
                .filter(|q| !q.is_trade())
                .map(|q| q.id)
                .collect()
        })
        .unwrap_or_default();
    let mut open = 0usize;
    let mut sweep: Vec<QuoteId> = Vec::new();
    for id in mine {
        let Some(q) = ctx
            .w
            .module_state::<MarketStore>()
            .and_then(|s| s.quote(id).cloned())
        else {
            continue;
        };
        if q.contract.is_some() || is_current(ctx, &q) {
            open = open.saturating_add(1);
        } else if sweep.len() < MAX_LAPSED_SWEEP_PER_CALL {
            sweep.push(id);
        }
    }
    for id in sweep {
        del_quote(ctx, id);
    }
    if open >= MAX_OPEN_QUOTES_PER_BUYER {
        return Err(MarketError::bad(
            "You hold as many open quotes as the market allows; take one up or let one lapse.",
        ));
    }
    // **Only OPEN offers count against the ceiling** (independent verify fold, R4). The
    // check counted trade-tape rows too, and the tape had no ceiling of its own: 64
    // traded pairs at `MAX_TRADES_PER_PAIR` filled `MAX_QUOTES`, and every buyer read
    // "The market is busy; try later." for ever with no new trade possible to evict
    // anything. The tape now has `MAX_TRADE_ROWS` and evicts oldest-settlement-first
    // across pairs, so a busy history can never refuse a fresh quote.
    if ctx
        .w
        .module_state::<MarketStore>()
        .is_some_and(|s| s.open_quotes() >= MAX_QUOTES.saturating_sub(MAX_TRADE_ROWS))
    {
        return Err(MarketError::bad("The market is busy; try later."));
    }
    // ---- B7: the recorded availability test -------------------------------
    let availability = match &entry.availability {
        AvailabilityRule::None | AvailabilityRule::Attested => None,
        rule @ AvailabilityRule::Test { .. } => {
            let outcome = crate::module::resolve_availability(ctx, rule, &entry.catalog);
            match outcome {
                None => {
                    return Err(MarketError::bad(
                        "Nothing on this server can say whether that can be got hold of.",
                    ));
                }
                Some(o) if !o.passed => {
                    // A failure refuses **without charging** — there is nothing to
                    // charge, because a quote is asked for before any money moves.
                    return Err(MarketError::bad(format!(
                        "Nobody can lay hands on one right now ({}). Nothing was charged.",
                        o.note
                    )));
                }
                Some(o) => Some(o),
            }
        }
    };
    let ttl = match req.window {
        QuoteWindow::Live => req.ttl_live.clamp(60, 7 * 86_400),
        QuoteWindow::Remote => req.ttl_remote.clamp(60, 30 * 86_400),
    };
    let now = ctx.w.now();
    let Some(id) = ctx
        .w
        .module_state_mut::<MarketStore>()
        .and_then(MarketStore::alloc_quote)
    else {
        return Err(MarketError::bad("The quote-id allocator is exhausted."));
    };
    put_quote(
        ctx,
        Quote {
            id,
            listing: listing.id,
            buyer,
            seller: listing.seller.clone(),
            catalog: listing.catalog.clone(),
            market: listing.market.clone(),
            visibility: listing.visibility,
            source: match listing.source {
                Supply::Market { source } => Some(source),
                Supply::Stock { .. }
                | Supply::Procurement { .. }
                | Supply::Lot { .. }
                | Supply::Wanted { .. } => None,
            },
            price: unit,
            quantity,
            // Nothing is booked until the deal is in force; `accept` stamps what it
            // actually spent, and the return leg subtracts exactly that (M5).
            booked_loss: Amount::ZERO,
            activated: false,
            self_dealt: false,
            definition_version: entry.definition_version,
            observed: ObservedRevisions {
                definition_version: entry.definition_version,
                listing: listing.revision,
                artefact: artefact_stamp(entry.representation, &listing.catalog),
                source: observed_source,
            },
            expires: EconomicDeadline {
                clock: EconomicClock::RealUtc,
                at: now.saturating_add(ttl),
            },
            availability,
            contract: None,
            settled: None,
            lot: None,
            drawn_at: None,
            cuts: Vec::new(),
            basis: Amount::ZERO,
            revision: 1,
        },
    );
    persist_meta(ctx);
    Ok(id)
}

/// The one refusal every exhausted budget gets — the **same** [`STALE_QUOTE`] string
/// `+market/accept` answers with (independent verify fold, D2).
///
/// It used to name the budget, which made `+market/quote` a free, unlimited solvency
/// oracle on a market maker's organization wallet: a refused quote writes no record and
/// consumes no per-buyer slot, so a stranger could poll `run out of cash` and watch an
/// org wallet cross zero — while `@account/show` on that very account answers them
/// `No such account.` under the economy's existence-hiding rule. It also split
/// D-E6-13: the same fact was `STALE_QUOTE` from one verb and a named reason from the
/// other. Staff keep the detail, on the tracing line, where a buyer is not.
fn maker_refusal(which: &str) -> String {
    tracing::debug!(budget = which, "market: a maker refused a quote");
    STALE_QUOTE.to_string()
}

/// What a sale of `quantity` at `unit` books against a maker's **loss** budget.
///
/// A maker selling *above* the catalogue's reference price books no loss; a maker
/// selling below books the difference, and its loss budget is what stops it selling the
/// world cheaply. The one place the arithmetic lives, so the spend leg and the quote's
/// own check cannot compute it two ways (independent verify fold, M5).
fn booked_loss(entry: &crate::record::CatalogEntry, unit: Amount, quantity: u64) -> Amount {
    let baseline = entry.baseline.unwrap_or(Amount::ZERO).minor();
    Amount::from_minor(
        baseline
            .saturating_sub(unit.minor())
            .max(0)
            .saturating_mul(i64::try_from(quantity).unwrap_or(i64::MAX)),
    )
}

/// Whether the maker's own cash account can still back a purchase at all.
fn can_cover_inventory(ctx: &Ctx<'_>, maker: &crate::record::MarketSource) -> bool {
    ctx.w
        .module_state::<EconomyStore>()
        .and_then(|s| {
            s.snapshot().account(maker.cash).map(|a| {
                a.posted_balance
                    .minor()
                    .saturating_sub(a.held_amount.minor())
                    > 0
            })
        })
        .unwrap_or(false)
}

/// A listing's unit price — **the one price every surface reads** (independent verify
/// fold, D4).
///
/// Stock and procurement ask their own price; a maker's is the listing price moved by
/// its declared spread, which is a **trading policy** and never a substitute for
/// funding (doc 10 7A). The adjusted price is deliberately never written onto the
/// `Listing` (it would drift on a re-budget), so `projection` and `commands` call this
/// instead: before they did, `+market/list` and the published band advertised a
/// pre-spread price nobody paid, while `last_trade` published the post-spread one the
/// deal actually executed at — the safe projection publishing a band that need not
/// contain its own last trade.
pub(crate) fn price_for(l: &Listing, store: Option<&MarketStore>) -> Amount {
    let Supply::Market { source } = l.source else {
        return l.price;
    };
    let Some(spread) = store.and_then(|s| s.source(source)).map(|s| s.spread) else {
        return l.price;
    };
    let moved = l
        .price
        .minor()
        .saturating_mul(1_000i64.saturating_add(spread))
        / 1_000;
    Amount::from_minor(moved.max(1))
}

/// **Take a quote up**: draft, offer, place the buyer's escrow, and let the vendor
/// accept programmatically (B6).
///
/// Returns the contract. Every refusal before `offer_contract` leaves the world
/// untouched, and a refused vendor accept cancels the draft, so a buyer never ends up
/// with a standing escrow against a deal the vendor could not take.
///
/// # Errors
///
/// [`MarketError`]; render it with [`crate::refuse_market`].
pub fn accept(ctx: &mut Ctx<'_>, id: QuoteId) -> Result<ContractId, MarketError> {
    accept_into(ctx, id, None)
}

/// **Take a quote up, naming where a lot-represented purchase is delivered** (E7r2 P1).
///
/// `destination` is `None` for every E6c representation and for a buyer who named
/// none; a `Supply::Lot` purchase needs one, because matter has to arrive somewhere and
/// `mush-inventory`'s endpoints are E3 vaults rather than a person. It must be an
/// endpoint the BUYER may take from — `+market/accept <quote>=<vault>` — and naming one
/// they may not is refused identically to naming one that does not exist.
///
/// # Errors
///
/// [`MarketError`]; render it with [`crate::refuse_market`].
pub fn accept_into(
    ctx: &mut Ctx<'_>,
    id: QuoteId,
    destination: Option<mush_inventory::EndpointRef>,
) -> Result<ContractId, MarketError> {
    accept_at(ctx, id, destination, crate::module::Presence::Here)
}

/// **Take a quote up under a declared presence** (doc 18 §4.3, cut IC7) — the one accept
/// path; [`accept_into`] is this with [`Presence::Here`](crate::Presence::Here), and
/// IC11's executor calls it with `Presence::Ship(hull)`.
///
/// Every refusal returns before `offer_contract`, so nothing is escrowed or charged:
/// the presence gate (its own words), the governed-endpoint rule
/// ([`PORT_ONLY_ENDPOINT`]), the capacity pre-check (the provider's words, or
/// [`CAPACITY_UNREADABLE`]), and for a Draw the storefront's available stock and the
/// composed `market` fact namespace.
///
/// # Errors
///
/// [`MarketError`]; render it with [`crate::refuse_market`].
pub fn accept_at(
    ctx: &mut Ctx<'_>,
    id: QuoteId,
    destination: Option<mush_inventory::EndpointRef>,
    presence: crate::module::Presence,
) -> Result<ContractId, MarketError> {
    let Some(mut q) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.quote(id).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_QUOTE));
    };
    if q.buyer != PrincipalId::Object(ctx.actor) {
        return Err(MarketError::NoStanding(NO_SUCH_QUOTE));
    }
    if q.contract.is_some() || q.is_trade() {
        return Err(MarketError::bad("That quote has already been taken up."));
    }
    // Doc 18 cut IC8 (lead ruling L14): a quote naming a standing bid is refused before
    // any write, in the bid's own words (`quote` never writes one; this is the belt).
    if ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.listing(q.listing))
        .is_some_and(|l| l.source.is_wanted())
    {
        return Err(MarketError::bad(WANTED_IS_A_BID));
    }
    // One refusal for every stale reason (D-E6-13): lapsed, re-priced, re-versioned,
    // sold out, halted, retired. Deterministic, and it re-prices nothing.
    if !is_current(ctx, &q) {
        return Err(MarketError::bad(STALE_QUOTE));
    }
    let Some(listing) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.listing(q.listing).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_QUOTE));
    };
    let Some(vendor) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor(listing.vendor).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_QUOTE));
    };
    let Some(entry) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&q.catalog).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_QUOTE));
    };
    let representation = entry.representation;
    // **DL-06 reaches the market** (independent verify fold, M4): a `Dissolving`
    // organization may settle what it already agreed to and may not agree to more; a
    // `Suspended` or `Closed` one may do neither. Fail closed, exactly as
    // `Vendor::is_trading` does.
    if !crate::api::org_may_trade(ctx.w, vendor.org) {
        return Err(MarketError::bad(STALE_QUOTE));
    }
    // ---- doc 18 cut IC7: what matter this sells, and the presence gate ----
    //
    // `lot_matter` is the one accessor (a hold-backed `Supply::Lot`, a drawn
    // `Supply::Stock` on a lot entry, or neither). A non-lot purchase meets the gate
    // here with no endpoint; a lot purchase meets it below, once its destination is
    // known. No gate is the pre-IC7 behaviour exactly.
    let matter = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.lot_matter(&listing));
    let gate = crate::module::venue_gate(ctx.w);
    if matter.is_none()
        && let Some(gate) = &gate
    {
        gate.admit(ctx.w, ctx.actor, &listing.market, None, presence)
            .map_err(|r| MarketError::bad(r.line))?;
    }
    // **A maker's budgets bound selling, not merely quoting** (independent verify fold,
    // M2). A quote reserves nothing (OPEN 4), so N quotes taken out against N listings
    // behind one maker each saw `volume_used`/`loss_used` at their pre-sale value, all
    // passed, and all charged: both budgets were exceeded without limit, because
    // `is_current` consults only `halted`. The value is the very one `charge_budgets`
    // computes a few lines below, so the check costs nothing and cannot disagree.
    let booked = booked_loss(&entry, q.price, q.quantity);
    if let Supply::Market { source } = listing.source {
        let now = ctx.w.now();
        let refused = ctx
            .w
            .module_state::<MarketStore>()
            .and_then(|s| s.source(source).cloned())
            .map_or(Some("gone"), |m| m.rolled(now).refuses(q.total(), booked));
        if let Some(why) = refused {
            tracing::debug!(
                budget = why,
                quote = q.id.0,
                "market: a maker refused a sale"
            );
            return Err(MarketError::bad(STALE_QUOTE));
        }
    }
    let Some(instrument) = crate::api::account_instrument(ctx, vendor.account) else {
        return Err(MarketError::NoStanding(NO_SUCH_QUOTE));
    };
    // **The goods leg, before any money moves** (independent verify fold, M1). The
    // market cannot know what a commodity entry settles in — that is the provider's
    // rule — so it asks the frozen table the question it will be asked at settlement,
    // and a provider that already knows it cannot deliver refuses the purchase here
    // rather than at settlement, where the money has gone. A provider that will not
    // answer (`None`) leaves the accept exactly as it was: this is a pre-check, never a
    // second authority.
    // ---- E7r2: P1 — the lot leg's own pre-check, before any money moves ----
    //
    // The same shape as `cannot_deliver` above and for the same reason: a purchase the
    // matter leg cannot honour must be refused with the buyer's money still theirs.
    // The destination is checked against `may_take_from_vault` — cut 2's exported
    // `may_take` — so naming a stranger's vault reads exactly like naming a vault that
    // does not exist, and the seller's own endpoint is refused because
    // `settle_sale` writes one `lot_census` key per endpoint (and a Draw from an
    // endpoint into itself moves nothing).
    let destination = match &matter {
        Some(m) => Some(lot_precheck(
            ctx,
            &q,
            &listing,
            m,
            destination,
            gate.as_ref(),
            presence,
        )?),
        None => None,
    };
    // Doc 18 cut IC7: a Draw's cutoff, computed before any money moves so an
    // unrepresentable instant refuses rather than taking the money for a deal whose
    // window cannot be written (checked arithmetic, real seconds).
    let accepted_at = ctx.w.now();
    let draw_by = match &matter {
        Some(LotMatter::Draw { .. }) => Some(
            accepted_at
                .checked_add(DRAW_WINDOW_SECS)
                .and_then(|t| t.checked_sub(DRAW_SETTLE_MARGIN_SECS))
                .ok_or_else(|| MarketError::bad(UNDELIVERABLE))?,
        ),
        Some(LotMatter::Hold { .. }) | None => None,
    };
    if let Some(why) = crate::module::cannot_deliver(ctx, &q, representation) {
        tracing::info!(
            quote = q.id.0,
            why,
            "market: a purchase was refused undeliverable"
        );
        return Err(MarketError::bad(UNDELIVERABLE));
    }
    let Some(buyer_account) = principal_account(ctx, &instrument, &q.buyer) else {
        return Err(MarketError::bad(
            "You have no account this market can be paid from.",
        ));
    };
    let spec = purchase_spec(
        &q,
        &listing,
        &vendor,
        representation,
        buyer_account,
        accepted_at,
        draw_by.map(|_| (q.id, accepted_at)),
    );
    let contract = mush_economy::offer_contract(ctx, spec).map_err(MarketError::Contract)?;
    // **The link is written before either acceptance** (independent verify fold, R9a).
    // It used to be written after `accept_contract_as` returned, so `by_quote_contract`
    // was empty for the whole of activation: the day any path settles inline at
    // activation — a `deterministic` contract whose only condition is an
    // already-satisfied `Before` is the candidate — `quote_of_contract` would answer
    // `None`, the observer would return early, and the trade row and the delivery would
    // be lost silently. On every failure path below `cancel_contract` emits a
    // `Cancelled` whose arm deletes the row, so writing early costs nothing.
    q.contract = Some(contract);
    q.booked_loss = booked;
    // **The public tape is not the seller's own to print** (independent verify fold,
    // M12), recorded here because affiliation is a fact about the world at the moment
    // the deal was struck, not about whoever folds the projection later.
    q.self_dealt = mush_economy::acts_for(ctx.w, ctx.actor, &q.seller);
    q.revision = q.revision.saturating_add(1);
    put_quote(ctx, q.clone());
    // The **buyer is the issuer** (E4b2's rule) and accepts first: that is what
    // places their consideration hold, under their own account's debit authority.
    match mush_economy::accept_contract(ctx, contract) {
        Ok(AcceptOutcome::Accepted { .. }) => {}
        Ok(AcceptOutcome::NeedsApproval(_)) => {
            let _ = mush_economy::cancel_contract(ctx, contract);
            return Err(MarketError::bad(BUY_NEEDS_APPROVAL));
        }
        Err(e) => {
            let _ = mush_economy::cancel_contract(ctx, contract);
            return Err(MarketError::bad(own_accept_refusal(
                &e,
                contract,
                BUY_NOT_AGREED,
            )));
        }
    }
    // The **vendor organization accepts programmatically** through E6b's
    // `with_party_caps` (G8) — or, since doc 18 cut IC7, through the composition's
    // content-resolved party-cap source, asked only after the table said no. Neither
    // grants any money authority whatever: the accept still runs `accept_core`, still
    // places the vendor's escrow (there is none here — it owes no consideration) under
    // the org account's own authority, and still refuses when the org cannot stand
    // behind the deal.
    let vendor_principal = PrincipalId::Organization(vendor.org);
    match mush_economy::accept_contract_as(ctx, contract, &vendor_principal) {
        Ok(AcceptOutcome::Accepted { .. }) => {}
        Ok(AcceptOutcome::NeedsApproval(_)) | Err(_) => {
            let _ = mush_economy::cancel_contract(ctx, contract);
            return Err(MarketError::bad(
                "That vendor cannot complete the sale right now. Nothing was agreed.",
            ));
        }
    }
    // ---- E7r2: P1 — the matter leg, at the SAME serialization point ----
    //
    // OPEN 1's default: the consume is ONE `Ctx::submit_authoritative` from
    // `+market/accept`, staged at the activation E6c already serializes on. Keeping
    // `+market` on the compatibility lane and deferring the consume to a sweep arm
    // would open a paid-but-unconsumed window the reservation was placed to close, and
    // a GOD tick may not stage an authoritative submission at all (the tick-cadence
    // rule). The money is escrowed and the contract is in force by the time this runs,
    // so **a lot is never consumed without the money leg**: every refusal above
    // returns before it, and a settle that never activates consumes nothing.
    //
    // Doc 18 cut IC7: the terms the money was taken for are frozen on the quote first
    // (`Quote::lot`, written by the final `put_quote` below), and the leg is built from
    // them — `consume` for a hold-backed sale, `draw` for a drawn one — carrying the
    // contract, so a re-stage (`restage_draw`) is byte-identical and the paid-leg
    // projection offers it only to this buyer.
    if let (Some(matter), Some(destination)) = (&matter, destination) {
        let paid = PaidLot {
            kind: matter.kind(),
            source: LotEndpoint(matter.source()),
            destination: LotEndpoint(destination),
            commodity: LotCommodity(matter.commodity().clone()),
            grade: match matter {
                LotMatter::Hold { grade, .. } => *grade,
                LotMatter::Draw { .. } => 0,
            },
            accepted_at,
            draw_by,
            marker: None,
        };
        if !crate::lot::stage(ctx, &paid_leg(&q, &paid, contract)) {
            // The money is in force and no leg was staged: a composition defect. A Draw
            // sale then never sees its fact and expires with a refund at its window; a
            // hold-backed one is §9 risk 3 (L14), and the log says so.
            tracing::error!(
                quote = q.id.0,
                contract = contract.0,
                "market: a paid lot leg could not be staged"
            );
        }
        q.lot = Some(paid);
    }
    // **The single serialization point** (OPEN 4): the listing's quantity decrements
    // when — and only when — the contract came into force. A quote reserved nothing,
    // so two quotes on the last unit are a race the economy already serialized, and
    // the loser reads `STALE_QUOTE` above with nothing charged.
    commit_quantity(ctx, &listing, q.quantity);
    // …and the maker's budgets are **consumed** at the same point, for the same
    // reason: a budget that is checked and never spent is not a budget, and a maker
    // would quote for ever however much it had already sold. It is the activation
    // rather than the settlement because that is the moment the deal is in force,
    // and because a cancelled or expired deal gives it back below.
    charge_budgets(ctx, &q, 1);
    // The deal is in force, and only now may a cancellation give anything back (R9).
    q.activated = true;
    q.revision = q.revision.saturating_add(1);
    put_quote(ctx, q);
    Ok(contract)
}

/// The paid lot leg one activated quote's frozen terms stage (doc 18 cut IC7): the same
/// leg at accept and at every re-stage, so its parameters and key are byte-identical.
///
/// Doc 18 cut IC10: a sale into a bid's leg (`Deliver`) acts on the **source** — the
/// seller's own endpoint — and names the house's storefront as its destination; the
/// buyer's legs act on the destination and name none.
fn paid_leg(q: &Quote, lot: &PaidLot, contract: ContractId) -> crate::lot::LotLeg {
    let (op, endpoint, destination) = match lot.kind {
        PaidKind::Hold => (crate::lot::LotOp::Consume, lot.destination.0, None),
        PaidKind::Draw => (crate::lot::LotOp::Draw, lot.destination.0, None),
        PaidKind::Deliver => (
            crate::lot::LotOp::Deliver,
            lot.source.0,
            Some(lot.destination.0),
        ),
    };
    crate::lot::LotLeg {
        op,
        listing: q.listing,
        endpoint,
        source: lot.source.0,
        destination,
        commodity: lot.commodity.0.clone(),
        quantity: q.quantity,
        grade: lot.grade,
        contract: Some(contract),
    }
}

/// **A lot purchase's pre-checks, before any money moves** (E7r2 P1, extended by doc 18
/// §4.3 in cut IC7). Returns the destination endpoint.
///
/// In order: the destination (the named one, else the gate's `endpoint_for`, else the
/// E7r2 refusal); custody of it and not the source; the gate's `admit` with the
/// endpoint; the governed-endpoint rule (lead 2e: only a Draw from one of the venue's
/// own port institutions delivers into a governed endpoint); the capacity pre-check;
/// and for a Draw the storefront's available stock and the composed `market` fact
/// namespace (a Draw sale whose fact nobody answers would never settle).
#[allow(clippy::too_many_arguments)]
fn lot_precheck(
    ctx: &Ctx<'_>,
    q: &Quote,
    listing: &Listing,
    matter: &LotMatter,
    destination: Option<mush_inventory::EndpointRef>,
    gate: Option<&std::sync::Arc<dyn crate::module::VenueGate>>,
    presence: crate::module::Presence,
) -> Result<mush_inventory::EndpointRef, MarketError> {
    let destination = destination
        .or_else(|| gate.and_then(|g| g.endpoint_for(ctx.w, ctx.actor, &listing.market, presence)));
    let Some(destination) = destination else {
        return Err(MarketError::bad(
            "Name the inventory endpoint this is delivered into: +market/accept \
             <quote>=<vault>.",
        ));
    };
    if !mush_inventory::may_take(ctx.w, ctx.actor, destination) || destination == matter.source() {
        return Err(MarketError::NoStanding(NO_SUCH_QUOTE));
    }
    if let Some(gate) = gate {
        gate.admit(
            ctx.w,
            ctx.actor,
            &listing.market,
            Some(destination),
            presence,
        )
        .map_err(|r| MarketError::bad(r.line))?;
        if gate.governed(ctx.w, destination)
            && !(matches!(matter, LotMatter::Draw { .. })
                && gate.port_seller(ctx.w, &listing.market, &listing.seller))
        {
            return Err(MarketError::bad(PORT_ONLY_ENDPOINT));
        }
    }
    // ---- the capacity pre-check (doc 18 §4.3 "The capacity function") ----
    if let Some(capacity) = crate::module::capacity_provider(ctx.w)
        && let Some(plan) = capacity.plan(ctx.w, destination)
    {
        if !plan.within_bounds() {
            return Err(MarketError::bad(CAPACITY_UNREADABLE));
        }
        let mut read_pool = |c: &mush_core::CanonicalId| {
            crate::lot::pool_balance(ctx, destination, c).map_err(|()| CapacityReadError)
        };
        match capacity.check(&plan, matter.commodity(), q.quantity, &mut read_pool) {
            Ok(()) => {}
            Err(CapacityRefusal::Refused { line, .. }) => return Err(MarketError::bad(line)),
            Err(CapacityRefusal::Unreadable) => {
                return Err(MarketError::bad(CAPACITY_UNREADABLE));
            }
        }
    }
    // ---- a Draw: the storefront holds it, and its fact can be answered ----
    if let LotMatter::Draw { source, commodity } = matter {
        let stocked = crate::lot::pool_available(ctx, *source, commodity)
            .is_ok_and(|available| available >= q.quantity);
        if !stocked || !mush_economy::has_fact_provider(ctx.w, crate::store::NS) {
            tracing::info!(
                quote = q.id.0,
                stocked,
                "market: a drawn purchase was refused undeliverable"
            );
            return Err(MarketError::bad(UNDELIVERABLE));
        }
    }
    Ok(destination)
}

/// **Ask again for a Draw's hand-over** (doc 18 §4.4, D-18-32; cut IC7, F11): re-stage
/// a taken-up Draw quote whose goods have not been seen to move, under the same
/// contract key with byte-identical parameters — the same request to the driver, so a
/// re-stage after a landing replays as a HIT and one after a refused prepare is
/// prepared again. Public because IC11's executor re-stages through it.
///
/// - `Ok(None)` — not a re-stageable Draw at all (not the actor's quote, not activated,
///   not a Draw, already drawn or settled, no contract, or the contract not `Active`):
///   the caller falls through to [`accept_at`] and its existing answers;
/// - `Err(DRAW_TOO_LATE)` — past the cutoff `draw_by` (real clock): the leg would not be
///   offered at drain anyway;
/// - `Err(<the undeliverable line>)` — the leg could not be staged;
/// - `Ok(Some(contract))` — staged.
///
/// # Errors
///
/// [`MarketError`]; render it with [`crate::refuse_market`].
pub fn restage_draw(ctx: &mut Ctx<'_>, id: QuoteId) -> Result<Option<ContractId>, MarketError> {
    let Some(q) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.quote(id).cloned())
    else {
        return Ok(None);
    };
    if q.buyer != PrincipalId::Object(ctx.actor)
        || !q.activated
        || q.drawn_at.is_some()
        || q.settled.is_some()
    {
        return Ok(None);
    }
    let (Some(lot), Some(contract)) = (q.lot.clone(), q.contract) else {
        return Ok(None);
    };
    let (PaidKind::Draw, Some(draw_by)) = (lot.kind, lot.draw_by) else {
        return Ok(None);
    };
    let active = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().contract(contract).map(|c| c.state))
        .is_some_and(|state| matches!(state, ContractState::Active));
    if !active {
        return Ok(None);
    }
    if ctx.w.now() > draw_by {
        return Err(MarketError::bad(DRAW_TOO_LATE));
    }
    if !crate::lot::stage(ctx, &paid_leg(&q, &lot, contract)) {
        return Err(MarketError::bad(UNDELIVERABLE));
    }
    Ok(Some(contract))
}

/// Take `quantity` off a listing, deleting it when it is exhausted.
///
/// **A standing bid is never deleted here** (doc 18 cut IC10, lead 2h-i): a sale into it
/// writes it back at `quantity − sold`, **including 0** — the exhausted bid stands, not
/// buying, as the readiness report's exhausted marker, and a refund restores it.
///
/// **A `Supply::Lot` listing is always exhausted by one sale** (E7r2 P1).
/// `ReservationBook::checked_consume` settles a hold **whole** — it is one record with
/// one amount, not a running counter — so the listing's reservation is gone the moment
/// any quantity is bought, and a surviving listing would advertise a hold that no
/// longer exists: the next buyer would pay, and the matter leg would refuse with their
/// money in force. `settle_sale` gives the seller the unsold remainder back as its own
/// tranche-rooted lot instead, and the seller re-lists it.
fn commit_quantity(ctx: &mut Ctx<'_>, listing: &Listing, quantity: u64) {
    if listing.source.lot().is_some() {
        crate::store::del_listing(ctx, listing.id);
        return;
    }
    let left = listing.quantity.saturating_sub(quantity);
    if left == 0 && !listing.source.is_wanted() {
        crate::store::del_listing(ctx, listing.id);
        return;
    }
    put_listing(
        ctx,
        Listing {
            quantity: left,
            revision: listing.revision.saturating_add(1),
            ..listing.clone()
        },
    );
}

/// Spend (`sign` = 1) or return (`sign` = -1) a maker's per-window budgets for one
/// deal, rolling the window forward first when it has lapsed.
///
/// Every budget in doc 10 7A is exhaustible, and exhaustible means *consumed by use*:
/// `volume_used` takes the whole trade, `loss_used` takes what the sale books against
/// the catalogue's own reference price (a sale **above** baseline books nothing). Both
/// saturate at zero on the way back, so a return can never mint budget.
///
/// **Both legs read the same denormalized number** (independent verify fold, M5). The
/// loss used to be recomputed from the catalogue's *live* `baseline` on the spend and
/// again on the return, while only `price` and `quantity` were on the quote — so any
/// pack reload in between made the return differ from the spend in whichever direction
/// the baseline moved. `Quote.booked_loss` is what the deal booked, and it is what the
/// deal gives back.
fn charge_budgets(ctx: &mut Ctx<'_>, q: &Quote, sign: i64) {
    let Some(source) = q.source else {
        return;
    };
    let Some(maker) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.source(source).cloned())
    else {
        return;
    };
    let total = q.total().minor();
    let loss = q.booked_loss.minor().max(0);
    let now = ctx.w.now();
    let rolled = maker.rolled(now);
    let next = crate::record::MarketSource {
        volume_used: Amount::from_minor(
            rolled
                .volume_used
                .minor()
                .saturating_add(total.saturating_mul(sign))
                .max(0),
        ),
        loss_used: Amount::from_minor(
            rolled
                .loss_used
                .minor()
                .saturating_add(loss.saturating_mul(sign))
                .max(0),
        ),
        revision: rolled.revision.saturating_add(1),
        ..rolled
    };
    crate::store::put_source(ctx, next);
}

/// Put `quantity` back on a listing whose deal did not complete.
fn restore_quantity(ctx: &mut Ctx<'_>, q: &Quote) {
    let Some(listing) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.listing(q.listing).cloned())
    else {
        // The listing was exhausted and deleted. Restoring it would re-mint a
        // document the vendor never asked for, so the goods simply return to stock
        // the way they left it: the vendor lists them again. Nothing is lost that the
        // vendor did not already have.
        return;
    };
    put_listing(
        ctx,
        Listing {
            quantity: listing.quantity.saturating_add(q.quantity),
            revision: listing.revision.saturating_add(1),
            ..listing
        },
    );
}

/// **The purchase contract**: money for goods, and no new settlement machinery.
///
/// One outcome row, exact payouts summing to the escrowed total (D-E6-6, answer 7),
/// and a deterministic condition set so nobody's word settles a shop purchase — the
/// journal's own state does.
fn purchase_spec(
    q: &Quote,
    listing: &Listing,
    vendor: &Vendor,
    representation: Representation,
    buyer_account: mush_economy::AccountId,
    now: i64,
    draw: Option<(QuoteId, i64)>,
) -> ContractSpec {
    let total = q.total();
    // Doc 18 cut IC7: a Draw sale's window is `DRAW_WINDOW_SECS` (real), and every
    // other sale's is `DEAL_WINDOW_SECS`, exactly as before.
    let deadline_at = match draw {
        Some((_, accepted_at)) => accepted_at.saturating_add(DRAW_WINDOW_SECS),
        None => now.saturating_add(DEAL_WINDOW_SECS),
    };
    // `Procurement` carries its **lead time as a `Before` deadline** (§5.4): the
    // delivery must land inside it, and past it the row stops holding and the
    // contract's own expiry refunds. Everything else is an immediate sale whose one
    // condition is the deal window itself.
    let (conditions, deliverables) = match (&listing.source, representation, listing.object) {
        // **A Draw sale settles only on proof the matter moved** (doc 18 §4.3, cut IC7,
        // L1): the window, and the `market.lot.drawn` fact the market answers from the
        // durable receipt the Draw writes in its own bundle. A Draw refused at prepare
        // writes none, so the fact stays unsatisfied, the contract never settles, and
        // its `Expiry::Refund` returns the escrow at the window with no page — nothing
        // is paid for nothing (§9 risk 3, closed for the Draw).
        _ if draw.is_some() => (
            vec![
                TypedCondition::Before {
                    deadline: EconomicDeadline {
                        clock: EconomicClock::RealUtc,
                        at: deadline_at,
                    },
                },
                TypedCondition::Fact {
                    fact: mush_economy::ExternalRef {
                        provider: SmolStr::new_static(crate::store::NS),
                        kind: SmolStr::new_static(crate::record::FACT_LOT_DRAWN),
                        id: SmolStr::new(draw.map_or_else(String::new, |(quote, at)| {
                            crate::lot::draw_fact_id(quote, at)
                        })),
                        revision: None,
                    },
                },
            ],
            Vec::new(),
        ),
        (Supply::Procurement { lead }, _, _) => {
            (vec![TypedCondition::Before { deadline: *lead }], Vec::new())
        }
        (_, Representation::Asset, Some(object)) => (
            vec![
                TypedCondition::TitleHeld {
                    object,
                    by: listing.seller.clone(),
                },
                TypedCondition::Before {
                    deadline: EconomicDeadline {
                        clock: EconomicClock::RealUtc,
                        at: now.saturating_add(DEAL_WINDOW_SECS),
                    },
                },
            ],
            // **Both parties must have accepted** before title moves (D-E3-12), which
            // is E4b2's own rule on a `TitleTransfer` and needs nothing from here.
            vec![Deliverable::TitleTransfer {
                object,
                from: listing.seller.clone(),
                to: q.buyer.clone(),
            }],
        ),
        _ => (
            vec![TypedCondition::Before {
                deadline: EconomicDeadline {
                    clock: EconomicClock::RealUtc,
                    at: now.saturating_add(DEAL_WINDOW_SECS),
                },
            }],
            Vec::new(),
        ),
    };
    let when: Vec<usize> = (0..conditions.len()).collect();
    let asset_consequences: Vec<usize> = (0..deliverables.len()).collect();
    let evidence: Vec<EvidenceKind> = conditions
        .iter()
        .map(|c| match c {
            TypedCondition::TitleHeld { .. } => EvidenceKind::TitleHeld,
            TypedCondition::After { .. } => EvidenceKind::After,
            TypedCondition::DepositInto { .. } => EvidenceKind::DepositInto,
            TypedCondition::Fact { .. } => EvidenceKind::Fact,
            TypedCondition::Attestation { .. } => EvidenceKind::Attestation,
            TypedCondition::Before { .. } => EvidenceKind::Before,
        })
        .collect();
    ContractSpec {
        name: SmolStr::new(format!("market purchase #{}", q.id.0)),
        scope: None,
        parties: vec![listing.seller.clone()],
        considerations: vec![ConsiderationSpec {
            account: buyer_account,
            amount: total,
        }],
        outcomes: vec![OutcomeTerms {
            name: SmolStr::new_static("delivered"),
            when,
            payouts: vec![PayoutShare {
                to: vendor.account,
                share: Share::Exact(total),
            }],
            expense_treatment: mush_economy::ExpenseTreatment::None,
            asset_consequences,
        }],
        remainder_to: buyer_account,
        conditions,
        // **No adjudicator.** Nobody's word settles a shop purchase: the journal's own
        // title and time state does, which is what makes it deterministic.
        adjudicator: None,
        evidence,
        deadline: Some(EconomicDeadline {
            clock: EconomicClock::RealUtc,
            at: deadline_at,
        }),
        // An uncompleted purchase simply refunds: nothing was taken, so there is
        // nothing to forfeit.
        on_expiry: Expiry::Refund,
        visibility: EconomicVisibility::Parties,
        roles: ContractRoles {
            author: Some(q.buyer.clone()),
            payer: Some(q.buyer.clone()),
            representative: None,
            facilitator: None,
        },
        deterministic: true,
        deliverables,
    }
}

// ---- +market/sell: the player-side offer to a vendor ------------------------

/// **`+market/sell <obj>=<vendor>`** — the player-side offer, on E6b's fence
/// consign/immediate shape (B5).
///
/// The vendor buys **custody** at its declared `buy_permille` of the catalogue
/// baseline: the deliverable is a `DepositInto` and there is no `TitleTransfer`
/// anywhere in the template, so the seller's title record and any dispute on it
/// survive the sale untouched (D-E6-10 generalized). The vendor accepts first, which
/// is what places the escrow (D-E6-9: money is committed before the goods move).
///
/// # Errors
///
/// [`MarketError`]; render it with [`crate::refuse_market`].
pub fn sell(
    ctx: &mut Ctx<'_>,
    instrument: &InstrumentId,
    object: ObjId,
    vendor_id: crate::ids::VendorId,
) -> Result<ContractId, MarketError> {
    let seller = ctx.actor;
    let Some(vendor) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor(vendor_id).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_VENDOR));
    };
    if !crate::api::is_staff(ctx.w, seller) && !crate::api::may_list_vendor(ctx.w, seller, &vendor)
    {
        return Err(MarketError::NoStanding(NO_SUCH_VENDOR));
    }
    if !vendor.is_trading() {
        return Err(MarketError::NoStanding(NO_SUCH_VENDOR));
    }
    // Doc 18 §4.3 (cut IC7): the presence gate, in its own words, before anything is
    // escrowed. No gate is the pre-IC7 behaviour exactly.
    if let Some(gate) = crate::module::venue_gate(ctx.w) {
        gate.admit(
            ctx.w,
            seller,
            &vendor.market,
            None,
            crate::module::Presence::Here,
        )
        .map_err(|r| MarketError::bad(r.line))?;
    }
    // The seller must actually be holding it — a world fact, not a gated one, exactly
    // as E6b's fence asks it.
    if ctx.w.get(object).and_then(|o| o.location) != Some(seller) {
        return Err(MarketError::NoStanding("No such asset."));
    }
    let Some(catalog) = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().asset(object).and_then(|a| a.catalog.clone()))
    else {
        return Err(MarketError::bad(
            "That has no catalogue entry, so no vendor can put a price on it.",
        ));
    };
    let Some(entry) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&catalog).cloned())
    else {
        return Err(MarketError::bad(
            "That has no catalogue entry, so no vendor can put a price on it.",
        ));
    };
    let Some(baseline) = entry.baseline.filter(|b| b.minor() > 0) else {
        return Err(MarketError::bad(
            "That has no reference price, so no vendor can put a price on it.",
        ));
    };
    let offer = Amount::from_minor(
        (baseline
            .minor()
            .saturating_mul(i64::from(vendor.buy_permille))
            / 1_000)
            .max(0),
    );
    if offer.minor() <= 0 {
        return Err(MarketError::bad("That vendor is not buying that."));
    }
    let Some(seller_account) = principal_account(ctx, instrument, &PrincipalId::Object(seller))
    else {
        return Err(MarketError::bad(
            "You need an account before a vendor can pay you.",
        ));
    };
    // **One object is one standing offer, and one seller holds a bounded number against
    // one shop** (independent verify fold, M9). The vendor escrows first (D-E6-9) and
    // the seller never accepts, so N offers on the same object all sat `Offered` with N
    // vendor holds standing: the economy's own duplicate-deposit lock is taken at
    // *activation*, which never arrived. At `buy_permille` of a high baseline that
    // froze a large share of a vendor's available balance for the whole deal window at
    // no cost to the offerer, and "a vendor that has run out simply cannot accept"
    // (M19, D-E6-15) then reads as insolvency to every honest buyer. It is the
    // per-buyer quote cap's shape, asked of the seller's own standing offers.
    let standing = standing_sells(ctx, seller, object, &vendor);
    if standing.duplicate {
        return Err(MarketError::bad(
            "You have already offered that to this vendor; the offer still stands.",
        ));
    }
    if standing.open >= MAX_OPEN_SELLS_PER_VENDOR {
        return Err(MarketError::bad(
            "You have as many offers standing with that vendor as it takes at once; let one \
             complete or cancel it.",
        ));
    }
    let now = ctx.w.now();
    let spec = ContractSpec {
        name: SmolStr::new(format!("market sale #{}", object.0)),
        scope: None,
        parties: vec![PrincipalId::Organization(vendor.org)],
        considerations: vec![ConsiderationSpec {
            account: vendor.account,
            amount: offer,
        }],
        outcomes: vec![OutcomeTerms {
            name: SmolStr::new_static("sold"),
            when: vec![0],
            payouts: vec![PayoutShare {
                to: seller_account,
                share: Share::Exact(offer),
            }],
            expense_treatment: mush_economy::ExpenseTreatment::None,
            asset_consequences: Vec::new(),
        }],
        remainder_to: vendor.account,
        conditions: vec![TypedCondition::DepositInto {
            object,
            vault: vendor.vault,
        }],
        adjudicator: None,
        evidence: vec![EvidenceKind::DepositInto],
        deadline: Some(EconomicDeadline {
            clock: EconomicClock::RealUtc,
            at: now.saturating_add(DEAL_WINDOW_SECS),
        }),
        on_expiry: Expiry::Refund,
        visibility: EconomicVisibility::Parties,
        roles: ContractRoles {
            author: Some(PrincipalId::Object(seller)),
            payer: Some(PrincipalId::Organization(vendor.org)),
            representative: None,
            facilitator: None,
        },
        deterministic: true,
        // A vendor buys CUSTODY, never title (D-E6-10 generalized): no
        // `TitleTransfer` is ever in this template.
        deliverables: Vec::new(),
    };
    let contract = mush_economy::offer_contract(ctx, spec).map_err(MarketError::Contract)?;
    let vendor_principal = PrincipalId::Organization(vendor.org);
    match mush_economy::accept_contract_as(ctx, contract, &vendor_principal) {
        Ok(AcceptOutcome::Accepted { .. }) => {}
        Ok(AcceptOutcome::NeedsApproval(_)) | Err(_) => {
            let _ = mush_economy::cancel_contract(ctx, contract);
            return Err(MarketError::bad(
                "That vendor cannot pay for it right now. Nothing was agreed.",
            ));
        }
    }
    Ok(contract)
}

/// What one seller already has standing against one vendor.
struct StandingSells {
    /// Whether one of them already names this very object.
    duplicate: bool,
    /// How many there are.
    open: usize,
}

/// Count one seller's non-terminal `+market/sell` offers against one vendor, and say
/// whether any of them already names `object` (independent verify fold, M9).
///
/// A `+market/sell` contract self-identifies with no market record at all: the seller
/// is its issuer and its one condition is a `DepositInto` naming that vendor's vault.
/// Bounded by the economy's own contract population, which its
/// `MAX_OPEN_CONTRACTS_PER_ISSUER` and `MAX_LIVE_CONTRACTS_PER_PARTY` caps already walk
/// the same way.
fn standing_sells(ctx: &Ctx<'_>, seller: ObjId, object: ObjId, vendor: &Vendor) -> StandingSells {
    let issuer = PrincipalId::Object(seller);
    let mut out = StandingSells {
        duplicate: false,
        open: 0,
    };
    let Some(store) = ctx.w.module_state::<EconomyStore>() else {
        return out;
    };
    for c in store
        .snapshot()
        .contracts()
        .filter(|c| !c.state.is_terminal() && c.issuer == issuer)
    {
        for condition in &c.consent.conditions {
            if let TypedCondition::DepositInto { object: o, vault } = condition
                && *vault == vendor.vault
            {
                out.open = out.open.saturating_add(1);
                out.duplicate = out.duplicate || *o == object;
            }
        }
    }
    out
}

// ---- doc 18 cut IC10: the lot sell arm — a sale into a standing bid ---------

/// The house is not buying: a closed vendor, an untradeable organization or an exhausted
/// bid (doc 18 cut IC10).
pub const NOT_BUYING: &str = "That house is not buying that, so nothing was sold.";
/// The bid's deadline has passed (cut IC10).
pub const BID_LAPSED: &str = "That bid has lapsed, so nothing was sold.";
/// More than the bid's remaining intake (cut IC10); `{remaining}` is filled in.
pub const OVER_INTAKE: &str = "That house wants only {remaining} more, so nothing was sold.";
/// A grade-floored bid: pool stock carries no grade (cut IC10).
pub const GRADED_BID: &str =
    "That house buys graded lots only, and pooled stock carries no grade, so nothing was sold.";
/// The seller already has an authoritative submission queued this step (cut IC10, IC8's
/// `Ctx::authoritative_pending` seam): the deliver leg could not be staged.
pub const SELL_BUSY: &str =
    "You have an order still being handled; try again in a moment. Nothing was sold.";
/// The engine's authoritative submit queue is full (doc 18 cut IC14 row 2): other
/// actors' submissions fill the engine's queue, so the deliver leg could not be staged;
/// `SELL_BUSY` would tell a seller with nothing pending that they had something
/// pending. Checked before any contract, as `SELL_BUSY` is (owner voice, cut IC14 O1).
pub const SELL_QUEUE_FULL: &str =
    "The exchange is busy right now; try again in a moment. Nothing was sold.";
/// The remedy line `+market/sell`'s lot arm sends after the sale line (doc 18 cut IC14
/// row 2; lead ruling L2): a leg the engine dropped is re-staged by `+market/accept`.
/// `{quote}` is the sale's quote id, its digits only (owner voice, cut IC14 O1).
pub const SELL_REMEDY: &str = "If the hand-over stalls, +market/accept {quote} asks again.";
/// No endpoint named and none the gate can place (cut IC10).
pub const NAME_SOURCE: &str =
    "Name the inventory endpoint the goods leave from: +market/sell <listing>=<qty> from=<vault>.";
/// A port house buys only out of a governed (berthed) hold (cut IC10).
pub const HOLD_ONLY_SOURCE: &str =
    "That house buys only out of a hold berthed at its counter, so nothing was sold.";
/// The sale's settlement cannot be built or answered here (cut IC10).
pub const UNSETTLEABLE: &str = "That sale cannot be settled here right now, so nothing was sold.";
/// The house cannot fund the whole sale (cut IC10).
pub const UNFUNDED: &str = "That house cannot pay for that right now, so nothing was sold.";
/// The source pool could not be read — refused, never read as zero (cut IC10).
pub const SOURCE_UNREADABLE: &str =
    "That endpoint's stock cannot be read right now, so nothing was sold.";
/// Over-selling (cut IC10); `{held}` and `{qty}` are filled in.
pub const SELL_SHORT: &str =
    "You have {held} of that there to sell, not {qty}, so nothing was sold.";
/// The seller's own acceptance needs an approval (cut IC10).
pub const SELL_NEEDS_APPROVAL: &str =
    "That sale needs an approval your account rule has not granted yet. Nothing was agreed.";
/// The buyer's own acceptance needs an approval (cut IC7's text, a const since doc 20b
/// RK-1).
pub const BUY_NEEDS_APPROVAL: &str =
    "That purchase needs an approval your account rule has not granted yet. Nothing was agreed.";
/// A party's own acceptance could not place its escrow: the account has too little
/// available, posted or already held (doc 20b RK-1; the buyer's usual refusal, which
/// read as [`BUY_NEEDS_APPROVAL`] before).
pub const CANNOT_COVER: &str =
    "Your account does not have enough available to cover that. Nothing was agreed.";
/// The buyer's own acceptance failed for any other reason (doc 20b RK-1).
pub const BUY_NOT_AGREED: &str = "That purchase cannot be agreed right now. Nothing was agreed.";
/// The seller's own acceptance failed for any other reason (doc 20b RK-1).
pub const SELL_NOT_AGREED: &str = "That sale cannot be agreed right now. Nothing was agreed.";

/// **The line a party hears when their own acceptance is refused** (doc 20b RK-1):
/// an escrow the account cannot fund is [`CANNOT_COVER`]; anything else is `other`,
/// with the economy's own refusal logged for the operator. A `NeedsApproval` outcome
/// is not a refusal and never reaches here.
fn own_accept_refusal(
    e: &mush_economy::ContractError,
    contract: ContractId,
    other: &'static str,
) -> &'static str {
    use mush_economy::{ContractError, EconomyError};
    match e {
        ContractError::Refused(
            EconomyError::InsufficientAvailable(_) | EconomyError::CreditViolation(_),
        ) => CANNOT_COVER,
        _ => {
            tracing::info!(
                contract = contract.0,
                error = %e,
                "market: a party's own acceptance was refused"
            );
            other
        }
    }
}
/// The house's acceptance (its escrow) failed (cut IC10).
pub const HOUSE_CANNOT_PAY: &str = "That house cannot pay for that right now. Nothing was agreed.";
/// The lot arm named a listing that is an ask, not a bid (cut IC10).
pub const SELLS_NOT_BUYS: &str =
    "That listing is for sale, not a bid; +market/sell sells only into a bid.";
/// The `+market/sell` grammar (cut IC10; doc 20 cut SM.5 adds the bare preview and
/// `=all`).
pub const SELL_USAGE: &str = "Usage: +market/sell <listing> to see what a bid takes, +market/sell <listing>=<qty>|all [from=<vault>] to sell into it, or +market/sell <object>=<vendor>.";
/// **The bare preview's first line** (doc 20 cut SM.5, §7.2): `{listing}`, `{name}`,
/// `{house}`, `{remaining}` (the bid's unfilled intake), `{price}`, `{held}`.
pub const SELL_PREVIEW: &str =
    "#{listing} {name}: {house} wants {remaining} more at {price} each; you hold {held} there.";
/// The preview's second line when `=all` would sell something: `{listing}`, `{take}`,
/// `{gross}`.
pub const SELL_PREVIEW_ALL: &str = "+market/sell {listing}=all sells {take} for {gross}.";
/// The preview's second line when `=all` would sell nothing.
pub const SELL_PREVIEW_NONE: &str = "+market/sell {listing}=all would sell nothing now.";
/// `=all` with nothing held at the source (doc 20 cut SM.5).
pub const SELL_NONE_HELD: &str = "You have none of that there to sell, so nothing was sold.";
/// The market's own sale line when no proceeds policy speaks (cut IC10). `{cuts}` is
/// `"; {label} {amount}"` per cut, and empty with none.
pub const SOLD_GENERIC: &str = "Sold {qty} × {name} to {shop} at {price} each: {gross}{cuts}; you receive {net} when the goods are in.";
/// A re-stage asked for after a sale's hand-over cutoff (cut IC10).
pub const DELIVER_TOO_LATE: &str =
    "That sale's hand-over window has closed; it lapses and the house is refunded. Nothing moved.";
/// A re-stage asked for after the leg already landed (doc 18 cut IC14 row 3; lead
/// ruling L3): the marker is written, the mirror has not swept it yet. Never "Nothing
/// moved" for goods that are in (owner voice, cut IC14 O1).
pub const DELIVER_LANDED: &str =
    "Those goods are already in the house's stores; the sale settles at the next sweep.";
/// A re-stage that cannot read the sale's marker (doc 18 cut IC14 row 3; lead ruling
/// L32): no lot supply, no marker or an unreadable applied family. Not "Nothing moved"
/// either, since it cannot know (owner voice, cut IC14 O1).
pub const DELIVER_UNCONFIRMED: &str =
    "That sale's hand-over cannot be checked right now; try again in a moment.";

/// A seller's `@contract/dispute` of their own sale into a bid (doc 18 cut IC14 row 7,
/// lead ruling L6; Q-18-64's default changed): refused, since the sale settles on the
/// goods' own marker and a dispute would only freeze the house's escrow and the bid's
/// intake (owner voice, cut IC14 O1).
pub const SALE_DISPUTE_REFUSED: &str = "A sale into a bid settles when the goods are in the \
     house's stores; ask staff if something is wrong with it.";

/// **The market's dispute veto** (doc 18 cut IC14 row 7, lead ruling L6): the
/// [`mush_economy::DisputeGuard`] a composition installs with
/// `EconomyModule::with_dispute_guard`. It refuses exactly one thing — the **seller**
/// of a sale into a bid (a [`PaidKind::Deliver`] quote) disputing that sale's
/// contract — with [`SALE_DISPUTE_REFUSED`]; every other contract, every other party,
/// and any world with no market store answers `None` (game-neutral). Staff's
/// `@contract/resolve` and an admin's dispute are never asked.
#[derive(Clone, Copy, Debug, Default)]
pub struct SaleDisputeGuard;

impl mush_economy::DisputeGuard for SaleDisputeGuard {
    fn refusal(
        &self,
        w: &mush_core::World,
        contract: &mush_economy::EconomicContract,
        party: &PrincipalId,
    ) -> Option<&'static str> {
        let store = w.module_state::<MarketStore>()?;
        let quote = store.quote(store.quote_of_contract(contract.id)?)?;
        let deliver = quote
            .lot
            .as_ref()
            .is_some_and(|lot| lot.kind == PaidKind::Deliver);
        (deliver && &quote.seller == party).then_some(SALE_DISPUTE_REFUSED)
    }
}

/// What one sale into a standing bid asks (doc 18 §4.3 "Bids that take real lots",
/// D-18-26; cut IC10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BidSaleRequest {
    /// The bid listing.
    pub listing: ListingId,
    /// How much to sell into it.
    pub quantity: u64,
    /// The endpoint the goods leave from; `None` asks the presence gate
    /// (`VenueGate::endpoint_for` — in a port, the berthed hold).
    pub source: Option<mush_inventory::EndpointRef>,
    /// How the seller is present: `Here` for the typed verb, `Ship(hull)` for IC11's
    /// executor.
    pub presence: crate::module::Presence,
}

/// What one sale into a standing bid made (cut IC10).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BidSale {
    /// The sale contract (the house's escrow sits on it).
    pub contract: ContractId,
    /// The sale's quote (seller = the player, buyer = the house).
    pub quote: QuoteId,
    /// `price × quantity`.
    pub gross: Amount,
    /// The proceeds policy's cuts, frozen on the quote.
    pub cuts: Vec<crate::record::ProceedsCut>,
    /// What the seller receives: `gross − Σ cuts`.
    pub net: Amount,
    /// The line the seller reads (the policy's, else [`SOLD_GENERIC`] filled in).
    pub line: String,
}

/// **Whether one sale into a bid's hand-over is still in flight** (doc 18 cut IC10,
/// lead ruling (d)): a Deliver quote, activated, unsettled, its contract non-terminal in
/// the economy's book, inside its cutoff, and its marker **not yet written** — read
/// exactly through `Ctx::action_state` on inventory's applied family, never the 60 s
/// mirror, so a landed leg is never subtracted twice. An unreadable marker counts as
/// pending, failing toward refusing a sale.
///
/// A **disputed** sale stays in flight past its cutoff (IC10 fixer, review F1): staff,
/// or the house's side, may `@contract/dispute` it (since doc 18 cut IC14 row 7 the
/// seller may not: [`SaleDisputeGuard`]), which freezes the house's escrow and the
/// bid's intake until staff or the parties resolve it, and the leg is no longer
/// offered. Its tonnes stay committed for as long
/// as the house's money is, so the same goods can never be sold a second time beside a
/// frozen sale; a resolution settles (stamped, so no longer pending) or refunds
/// (terminal, the bid restored).
pub(crate) fn delivery_pending(ctx: &Ctx<'_>, q: &Quote) -> bool {
    let (Some(lot), Some(contract)) = (&q.lot, q.contract) else {
        return false;
    };
    if lot.kind != PaidKind::Deliver || !q.activated || q.settled.is_some() {
        return false;
    }
    let Some(state) = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().contract(contract).map(|c| c.state))
    else {
        return false;
    };
    if state.is_terminal() {
        return false;
    }
    let disputed = matches!(state, ContractState::Disputed);
    if !disputed && lot.draw_by.is_none_or(|by| ctx.w.now() > by) {
        return false;
    }
    let Some(marker) = lot.marker.as_ref() else {
        return true;
    };
    let Some(families) = crate::module::lot_supply(ctx.w) else {
        return true;
    };
    !matches!(ctx.action_state(&families.applied, marker), Ok(Some(_)))
}

/// A vendor account's available balance — posted minus held, the list row's own
/// arithmetic — or `None` when it cannot be read.
fn available_of(ctx: &Ctx<'_>, account: mush_economy::AccountId) -> Option<i64> {
    ctx.w
        .module_state::<EconomyStore>()?
        .snapshot()
        .account(account)
        .map(|a| {
            a.posted_balance
                .minor()
                .saturating_sub(a.held_amount.minor())
        })
}

/// The sale contract's name: `market sale #{quote} to bid #{listing}`, with the policy's
/// memo appended as ` ({memo})` when it fits [`mush_economy::MAX_CONTRACT_NAME_BYTES`]
/// (dropped when it does not), and `market sale #{quote}` if even that is over.
fn sale_name(quote: QuoteId, listing: ListingId, memo: Option<&SmolStr>) -> SmolStr {
    let base = format!("market sale #{} to bid #{}", quote.0, listing.0);
    if let Some(memo) = memo {
        let with = format!("{base} ({memo})");
        if with.len() <= mush_economy::MAX_CONTRACT_NAME_BYTES {
            return SmolStr::new(with);
        }
    }
    if base.len() <= mush_economy::MAX_CONTRACT_NAME_BYTES {
        SmolStr::new(base)
    } else {
        SmolStr::new(format!("market sale #{}", quote.0))
    }
}

/// **What the seller holds for sale at `source`** (cut IC10's step 11, factored by doc
/// 20 cut SM.5 so the sale and its preview read it one way): the source pool of
/// `commodity` less every sale from it still in flight (`delivery_pending`).
///
/// # Errors
///
/// [`SOURCE_UNREADABLE`] when the pool cannot be read — refused, never read as zero.
fn held_for_sale(
    ctx: &Ctx<'_>,
    actor: ObjId,
    source: mush_inventory::EndpointRef,
    commodity: &mush_core::CanonicalId,
) -> Result<u64, MarketError> {
    let Ok(pool) = crate::lot::pool_available(ctx, source, commodity) else {
        return Err(MarketError::bad(SOURCE_UNREADABLE));
    };
    let seller = PrincipalId::Object(actor);
    let in_flight = ctx
        .w
        .module_state::<MarketStore>()
        .map(|s| {
            s.quotes_of_seller(&seller)
                .filter(|q| {
                    q.lot
                        .as_ref()
                        .is_some_and(|l| l.source.0 == source && l.commodity.0 == *commodity)
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|q| delivery_pending(ctx, q))
        .fold(0_u64, |sum, q| sum.saturating_add(q.quantity));
    Ok(pool.saturating_sub(in_flight))
}

/// What a sale into a standing bid could take now (doc 20 cut SM.5, §7.2): read-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BidTake {
    /// The bid listing.
    pub listing: ListingId,
    /// The good's name as the listing line prints it.
    pub name: String,
    /// The house's name (the vendor's shop name).
    pub house: String,
    /// The bid's price per unit.
    pub price: Amount,
    /// The bid's unfilled intake (its listing quantity).
    pub remaining: u64,
    /// What the seller holds for sale at the source (pool less in-flight sales).
    pub held: u64,
    /// How many whole units the house can fund at the price.
    pub house_units: u64,
    /// The source the sale would default to (or the one named).
    pub source: mush_inventory::EndpointRef,
}

impl BidTake {
    /// What `=all` sells: `min(remaining, held, house_units)`.
    #[must_use]
    pub fn take(&self) -> u64 {
        self.remaining.min(self.held).min(self.house_units)
    }
}

/// **What a sale into a standing bid could take now** (doc 20 cut SM.5, §7.2; lead
/// ruling 8): exactly [`sell_into_bid`]'s read-only steps 1 (the listing, viewable,
/// a bid), 2 (the house trades), 3 (the deadline), 5 (the grade floor), 7 (the
/// source, named or the gate's placement, held by the seller), 8 (the presence gate
/// and the hold-only rule) and 11 (the pool less every in-flight sale), with the same
/// refusals — so a preview never says more than a sale would — and the house's
/// fundable units from its available funds over the price. Writes nothing.
///
/// # Errors
///
/// [`MarketError`], exactly as [`sell_into_bid`] refuses at those steps.
pub fn bid_take(
    ctx: &Ctx<'_>,
    listing: ListingId,
    source: Option<mush_inventory::EndpointRef>,
    presence: crate::module::Presence,
) -> Result<BidTake, MarketError> {
    let actor = ctx.actor;
    // 1.
    let Some(listing) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.listing(listing).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    };
    if !may_view_listing(ctx.w, actor, &listing) {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    }
    let Supply::Wanted {
        vault,
        commodity,
        min_grade,
    } = listing.source.clone()
    else {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    };
    let commodity = commodity.0;
    // 2.
    let Some(vendor) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor(listing.vendor).cloned())
    else {
        return Err(MarketError::bad(NOT_BUYING));
    };
    if !vendor.is_trading() || !crate::api::org_may_trade(ctx.w, vendor.org) {
        return Err(MarketError::bad(NOT_BUYING));
    }
    // 3.
    if crate::projection::lapsed(listing.expires.at, listing.expires.clock, ctx.w.now()) {
        return Err(MarketError::bad(BID_LAPSED));
    }
    // 5.
    if min_grade > 0 {
        return Err(MarketError::bad(GRADED_BID));
    }
    // 7.
    let gate = crate::module::venue_gate(ctx.w);
    let Some(source) = source.or_else(|| {
        gate.as_ref()
            .and_then(|g| g.endpoint_for(ctx.w, actor, &listing.market, presence))
    }) else {
        return Err(MarketError::bad(NAME_SOURCE));
    };
    if !mush_inventory::may_take(ctx.w, actor, source) || source.vault() == vault {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    }
    // 8.
    if let Some(gate) = &gate {
        gate.admit(ctx.w, actor, &listing.market, Some(source), presence)
            .map_err(|r| MarketError::bad(r.line))?;
        if gate.port_seller(ctx.w, &listing.market, &listing.seller)
            && !gate.governed(ctx.w, source)
        {
            return Err(MarketError::bad(HOLD_ONLY_SOURCE));
        }
    }
    // 11.
    let held = held_for_sale(ctx, actor, source, &commodity)?;
    // The house's fundable units (step 10's funds, read as a count).
    let price = listing.price.minor();
    let house_units = match available_of(ctx, vendor.account) {
        Some(available) if price > 0 && available > 0 => {
            u64::try_from(available / price).unwrap_or(0)
        }
        _ => 0,
    };
    // The listing line's own name: the catalogue entry's, else its code.
    let name = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&listing.catalog).map(|e| e.name.to_string()))
        .unwrap_or_else(|| listing.catalog.code.to_string());
    Ok(BidTake {
        listing: listing.id,
        name,
        house: vendor.name.to_string(),
        price: listing.price,
        remaining: listing.quantity,
        held,
        house_units,
        source,
    })
}

/// **The one sale-into-a-bid path** (doc 18 §4.3 "Bids that take real lots", D-18-26;
/// cut IC10): the typed `+market/sell <listing>=<qty>` with `Presence::Here`, and IC11's
/// executor with `Presence::Ship(hull)` and `source: None` (lead ruling (c)).
///
/// **Every refusal returns before `offer_contract`** (lead ruling (d)): the listing and
/// its bid shape, the house, the deadline, the quantity against the remaining intake,
/// the grade floor, a queued submission, the source and its custody, the presence gate
/// and the port's hold-only rule, the composed `inventory` fact namespace, the house's
/// funds, the source pool less every in-flight sale from it (`delivery_pending`), the
/// seller's account, the standing-sales and store caps, and the proceeds policy with its
/// split validated. Then, in `accept_at`'s order: the quote id, the window and cutoff,
/// the marker id derived from the contract the offer is about to mint
/// (`EconomyStore::peek_next_contract`, verified after the offer), the offer, the quote
/// written before either acceptance (R9a), the seller's in-verb acceptance (doc 18
/// §4.3's "no seller accept" is no step **asked** of the seller), the house's
/// programmatic acceptance (its escrow and the activation), the one deliver leg staged
/// through `paid_leg`, the bid's intake committed, and the quote activated. **Money
/// never moves before the matter**: the contract is gated on
/// `Fact{inventory, inventory.transferred, <marker>}`.
///
/// # Errors
///
/// [`MarketError`]; render it with [`crate::refuse_market`].
#[allow(clippy::too_many_lines)]
pub fn sell_into_bid(ctx: &mut Ctx<'_>, req: &BidSaleRequest) -> Result<BidSale, MarketError> {
    let actor = ctx.actor;
    // 1. The listing, viewable, and a bid — one answer for absent and unviewable.
    let Some(listing) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.listing(req.listing).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    };
    if !may_view_listing(ctx.w, actor, &listing) {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    }
    let Supply::Wanted {
        vault,
        commodity,
        min_grade,
    } = listing.source.clone()
    else {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    };
    let commodity = commodity.0;
    // 2. The house trades.
    let Some(vendor) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor(listing.vendor).cloned())
    else {
        return Err(MarketError::bad(NOT_BUYING));
    };
    if !vendor.is_trading() || !crate::api::org_may_trade(ctx.w, vendor.org) {
        return Err(MarketError::bad(NOT_BUYING));
    }
    // 3. The deadline.
    if crate::projection::lapsed(listing.expires.at, listing.expires.clock, ctx.w.now()) {
        return Err(MarketError::bad(BID_LAPSED));
    }
    // 4. The quantity, against the intake still wanted.
    let quantity = req.quantity;
    if quantity == 0 || quantity > crate::record::MAX_QUANTITY {
        return Err(MarketError::bad("That is not a quantity this market buys."));
    }
    if listing.quantity == 0 {
        return Err(MarketError::bad(NOT_BUYING));
    }
    if quantity > listing.quantity {
        return Err(MarketError::bad(
            OVER_INTAKE.replace("{remaining}", &listing.quantity.to_string()),
        ));
    }
    // 5. Pool stock carries no grade (tacsim posts 0).
    if min_grade > 0 {
        return Err(MarketError::bad(GRADED_BID));
    }
    // 6. IC8's seam: a queued submission this step means the leg could not be staged.
    if ctx.authoritative_pending(actor) {
        return Err(MarketError::bad(SELL_BUSY));
    }
    // 6b. Doc 18 cut IC14 row 2: a full engine queue would drop the deliver leg
    // AFTER the house's escrow, so refuse before any contract. The seam counts the
    // queued actors and every submission deferred earlier in this outbox (the
    // executor sells as several pilots in one `fire_due`).
    if ctx.authoritative_queue_full() {
        return Err(MarketError::bad(SELL_QUEUE_FULL));
    }
    // 7. The source: named, else the gate's placement; held by the seller; not the
    // house's own vault.
    let gate = crate::module::venue_gate(ctx.w);
    let Some(source) = req.source.or_else(|| {
        gate.as_ref()
            .and_then(|g| g.endpoint_for(ctx.w, actor, &listing.market, req.presence))
    }) else {
        return Err(MarketError::bad(NAME_SOURCE));
    };
    if !mush_inventory::may_take(ctx.w, actor, source) || source.vault() == vault {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    }
    // 8. The presence gate in its own words (NOT_IN_PORT, HOLD_UNDER_WAY,
    // HOLD_ELSEWHERE — the cross-port sale — and NOT_YOUR_CHARTER), then the port's
    // hold-only rule.
    if let Some(gate) = &gate {
        gate.admit(ctx.w, actor, &listing.market, Some(source), req.presence)
            .map_err(|r| MarketError::bad(r.line))?;
        if gate.port_seller(ctx.w, &listing.market, &listing.seller)
            && !gate.governed(ctx.w, source)
        {
            return Err(MarketError::bad(HOLD_ONLY_SOURCE));
        }
    }
    // 9. Somebody answers the fact the contract is gated on.
    if !mush_economy::has_fact_provider(ctx.w, mush_inventory::NS) {
        return Err(MarketError::bad(UNSETTLEABLE));
    }
    // 10. The house can fund the whole sale.
    let gross = i64::try_from(quantity)
        .ok()
        .and_then(|q| listing.price.minor().checked_mul(q))
        .map(Amount::from_minor)
        .ok_or_else(|| MarketError::bad(UNSETTLEABLE))?;
    if available_of(ctx, vendor.account).is_none_or(|available| available < gross.minor()) {
        return Err(MarketError::bad(UNFUNDED));
    }
    // 11. Over-selling: the pool, less every sale from it still in flight.
    let held = held_for_sale(ctx, actor, source, &commodity)?;
    let seller = PrincipalId::Object(actor);
    if held < quantity {
        return Err(MarketError::bad(
            SELL_SHORT
                .replace("{held}", &held.to_string())
                .replace("{qty}", &quantity.to_string()),
        ));
    }
    // 12. The seller's account, in the house's instrument.
    let Some(instrument) = crate::api::account_instrument(ctx, vendor.account) else {
        return Err(MarketError::bad(UNSETTLEABLE));
    };
    let Some(seller_account) = principal_account(ctx, &instrument, &seller) else {
        return Err(MarketError::bad(
            "You need an account before a vendor can pay you.",
        ));
    };
    // 13. Standing sales against this house, and the store's own caps.
    let house = PrincipalId::Organization(vendor.org);
    let standing = ctx
        .w
        .module_state::<MarketStore>()
        .map(|s| {
            s.quotes_of_seller(&seller)
                .filter(|q| {
                    q.buyer == house
                        && q.activated
                        && q.settled.is_none()
                        && q.lot.as_ref().is_some_and(|l| l.kind == PaidKind::Deliver)
                })
                .filter_map(|q| q.contract)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .filter(|c| {
            ctx.w
                .module_state::<EconomyStore>()
                .and_then(|s| s.snapshot().contract(*c).map(|k| !k.state.is_terminal()))
                .unwrap_or(false)
        })
        .count();
    if standing >= MAX_OPEN_SELLS_PER_VENDOR {
        return Err(MarketError::bad(
            "You have as many offers standing with that vendor as it takes at once; let one \
             complete or cancel it.",
        ));
    }
    if ctx
        .w
        .module_state::<MarketStore>()
        .is_none_or(|s| s.quotes.len() >= MAX_QUOTES)
    {
        return Err(MarketError::bad("The market is busy; try later."));
    }
    // 14. The proceeds policy, its split validated (unwired: no cuts, basis 0).
    let proceeds = match crate::module::proceeds_policy(ctx.w) {
        None => crate::module::Proceeds::default(),
        Some(policy) => {
            let terms = crate::module::SaleTerms {
                seller: actor,
                source,
                house: &listing.seller,
                market: &listing.market,
                listing: listing.id,
                commodity: &commodity,
                quantity,
                price: listing.price,
                gross,
            };
            let p = policy
                .proceeds(ctx.w, &terms)
                .map_err(|r| MarketError::bad(r.line))?;
            if !proceeds_valid(ctx, &p, &instrument, gross) {
                tracing::error!(
                    listing = listing.id.0,
                    "market: a proceeds policy returned a split the market cannot settle"
                );
                return Err(MarketError::bad(UNSETTLEABLE));
            }
            p
        }
    };
    let cut_total = proceeds
        .cuts
        .iter()
        .fold(0_i64, |sum, c| sum.saturating_add(c.amount.minor()));
    let net = Amount::from_minor(gross.minor().saturating_sub(cut_total));
    // 15. The quote id, the window and cutoff, and the marker id.
    let Some(quote) = ctx
        .w
        .module_state_mut::<MarketStore>()
        .and_then(MarketStore::alloc_quote)
    else {
        return Err(MarketError::bad("The market is busy; try later."));
    };
    persist_meta(ctx);
    let accepted_at = ctx.w.now();
    let (Some(cutoff), Some(deadline)) = (
        accepted_at
            .checked_add(DRAW_WINDOW_SECS)
            .and_then(|t| t.checked_sub(DRAW_SETTLE_MARGIN_SECS)),
        accepted_at.checked_add(DRAW_WINDOW_SECS),
    ) else {
        return Err(MarketError::bad(UNSETTLEABLE));
    };
    let Some(peeked) = ctx
        .w
        .module_state::<EconomyStore>()
        .map(EconomyStore::peek_next_contract)
    else {
        return Err(MarketError::bad(UNSETTLEABLE));
    };
    let Ok(marker) = mush_inventory::derived_marker_id(
        ctx.w.world_ic_now().at,
        &serde_json::json!({
            "kind": "market.deliver",
            "contract": peeked,
            "listing": listing.id.0,
            "quantity": quantity,
        }),
    ) else {
        return Err(MarketError::bad(UNSETTLEABLE));
    };
    // 16. The offer.
    let spec = sale_spec(
        &SaleShape {
            quote,
            listing: &listing,
            vendor: &vendor,
            seller: actor,
            seller_account,
            gross,
            net,
            cuts: &proceeds.cuts,
            deadline,
            marker: &marker,
        },
        proceeds.memo.as_ref(),
    );
    let contract = mush_economy::offer_contract(ctx, spec).map_err(MarketError::Contract)?;
    if contract.0 != peeked {
        let _ = mush_economy::cancel_contract(ctx, contract);
        tracing::error!(
            contract = contract.0,
            peeked,
            "market: a sale contract was minted under another id than the one its marker names"
        );
        return Err(MarketError::bad(UNSETTLEABLE));
    }
    // 17. The quote, written before either acceptance (R9a).
    let paid = PaidLot {
        kind: PaidKind::Deliver,
        source: LotEndpoint(source),
        destination: LotEndpoint(mush_inventory::EndpointRef(vault)),
        commodity: LotCommodity(commodity.clone()),
        grade: 0,
        accepted_at,
        draw_by: Some(cutoff),
        marker: Some(marker),
    };
    let mut q = Quote {
        id: quote,
        listing: listing.id,
        buyer: house.clone(),
        seller: seller.clone(),
        catalog: listing.catalog.clone(),
        market: listing.market.clone(),
        visibility: listing.visibility,
        source: None,
        price: listing.price,
        quantity,
        booked_loss: Amount::ZERO,
        activated: false,
        self_dealt: mush_economy::acts_for(ctx.w, actor, &house),
        definition_version: listing.definition_version,
        observed: ObservedRevisions {
            definition_version: listing.definition_version,
            listing: listing.revision,
            artefact: None,
            source: None,
        },
        expires: EconomicDeadline {
            clock: EconomicClock::RealUtc,
            at: deadline,
        },
        availability: None,
        contract: Some(contract),
        settled: None,
        lot: Some(paid.clone()),
        drawn_at: None,
        cuts: proceeds.cuts.clone(),
        basis: proceeds.basis,
        revision: 1,
    };
    put_quote(ctx, q.clone());
    // 18. The seller accepts in the verb — the economy needs every party, the issuer
    // included (doc 18 §4.3's "no seller accept" is no step asked of the seller).
    match mush_economy::accept_contract(ctx, contract) {
        Ok(AcceptOutcome::Accepted { .. }) => {}
        Ok(AcceptOutcome::NeedsApproval(_)) => {
            let _ = mush_economy::cancel_contract(ctx, contract);
            return Err(MarketError::bad(SELL_NEEDS_APPROVAL));
        }
        Err(e) => {
            let _ = mush_economy::cancel_contract(ctx, contract);
            return Err(MarketError::bad(own_accept_refusal(
                &e,
                contract,
                SELL_NOT_AGREED,
            )));
        }
    }
    // 19. The house accepts programmatically: its escrow, and the activation.
    //
    // Doc 18 cut IC14 row 66 (documented, no behaviour change): the house is a party
    // to every sale into its bid, and the economy's `MAX_LIVE_CONTRACTS_PER_PARTY`
    // counts a sale as one of its live contracts while it is `Active` or
    // `ReadyToSettle`, so the cap bounds a house's concurrent sales. The cap is checked
    // when the contract is offered (step 16's `offer_contract`), not here: past it
    // the seller hears the economy's own cap refusal and nothing is drafted (doc 20b
    // RK-1 corrected this note, which said the accept below refused).
    match mush_economy::accept_contract_as(ctx, contract, &house) {
        Ok(AcceptOutcome::Accepted { .. }) => {}
        Ok(AcceptOutcome::NeedsApproval(_)) | Err(_) => {
            let _ = mush_economy::cancel_contract(ctx, contract);
            return Err(MarketError::bad(HOUSE_CANNOT_PAY));
        }
    }
    // 20. The one deliver leg, through the builder every re-stage uses.
    if !crate::lot::stage(ctx, &paid_leg(&q, &paid, contract)) {
        // The money is escrowed and no leg was staged: the seller asks again with
        // `+market/accept <quote>`, or the contract expires with a refund at its
        // deadline (the Draw precedent).
        tracing::error!(
            quote = quote.0,
            contract = contract.0,
            "market: a sale's deliver leg could not be staged"
        );
    }
    // 21. The bid's intake, committed at activation (it stands at 0 when exhausted).
    commit_quantity(ctx, &listing, quantity);
    // 22. In force.
    q.activated = true;
    q.revision = q.revision.saturating_add(1);
    put_quote(ctx, q);
    let line = proceeds.line.clone().unwrap_or_else(|| {
        sold_generic(
            ctx,
            &listing,
            &vendor,
            &instrument,
            quantity,
            gross,
            net,
            &proceeds.cuts,
        )
    });
    Ok(BidSale {
        contract,
        quote,
        gross,
        cuts: proceeds.cuts,
        net,
        line,
    })
}

/// Whether a proceeds policy's answer is one the market can settle exactly (cut IC10,
/// lead ruling (b)): at most [`crate::MAX_PROCEEDS_CUTS`] cuts, every one well-formed
/// and paying an existing account in the sale's instrument, Σ cuts ≤ gross, and the
/// line and memo inside their caps.
fn proceeds_valid(
    ctx: &Ctx<'_>,
    p: &crate::module::Proceeds,
    instrument: &InstrumentId,
    gross: Amount,
) -> bool {
    if p.cuts.len() > crate::record::MAX_PROCEEDS_CUTS
        || p.line
            .as_ref()
            .is_some_and(|l| l.len() > crate::module::MAX_SALE_LINE_BYTES)
        || p.memo
            .as_ref()
            .is_some_and(|m| m.len() > crate::module::MAX_PROCEEDS_MEMO_BYTES)
        || p.basis.minor() < 0
    {
        return false;
    }
    let mut total: i64 = 0;
    for cut in &p.cuts {
        if cut.check().is_err()
            || crate::api::account_instrument(ctx, cut.to).as_ref() != Some(instrument)
        {
            return false;
        }
        let Some(next) = total.checked_add(cut.amount.minor()) else {
            return false;
        };
        total = next;
    }
    total <= gross.minor()
}

/// The parts one sale contract is built from (cut IC10).
struct SaleShape<'a> {
    quote: QuoteId,
    listing: &'a Listing,
    vendor: &'a Vendor,
    seller: ObjId,
    seller_account: mush_economy::AccountId,
    gross: Amount,
    net: Amount,
    cuts: &'a [crate::record::ProceedsCut],
    deadline: i64,
    marker: &'a SmolStr,
}

/// **The sale contract** (doc 18 §4.3, D-18-26; cut IC10): the house escrows the gross,
/// one outcome pays the seller's net and each cut `Exact`, the remainder (none, when the
/// split is exact) returns to the house, and the conditions are the window and
/// `Fact{inventory, inventory.transferred, <marker>}` — so the house's money is paid out
/// only once the goods are in its stores. Deterministic, refunded at the deadline.
fn sale_spec(s: &SaleShape<'_>, memo: Option<&SmolStr>) -> ContractSpec {
    let mut payouts = Vec::new();
    if s.net.minor() > 0 {
        payouts.push(PayoutShare {
            to: s.seller_account,
            share: Share::Exact(s.net),
        });
    }
    for cut in s.cuts {
        payouts.push(PayoutShare {
            to: cut.to,
            share: Share::Exact(cut.amount),
        });
    }
    let deadline = EconomicDeadline {
        clock: EconomicClock::RealUtc,
        at: s.deadline,
    };
    ContractSpec {
        name: sale_name(s.quote, s.listing.id, memo),
        scope: None,
        parties: vec![PrincipalId::Organization(s.vendor.org)],
        considerations: vec![ConsiderationSpec {
            account: s.vendor.account,
            amount: s.gross,
        }],
        outcomes: vec![OutcomeTerms {
            name: SmolStr::new_static("delivered"),
            when: vec![0, 1],
            payouts,
            expense_treatment: mush_economy::ExpenseTreatment::None,
            asset_consequences: Vec::new(),
        }],
        remainder_to: s.vendor.account,
        conditions: vec![
            TypedCondition::Before { deadline },
            TypedCondition::Fact {
                fact: mush_economy::ExternalRef {
                    provider: SmolStr::new_static(mush_inventory::NS),
                    kind: SmolStr::new_static(mush_inventory::FACT_TRANSFERRED),
                    id: s.marker.clone(),
                    revision: None,
                },
            },
        ],
        adjudicator: None,
        evidence: vec![EvidenceKind::Before, EvidenceKind::Fact],
        deadline: Some(deadline),
        on_expiry: Expiry::Refund,
        visibility: EconomicVisibility::Parties,
        roles: ContractRoles {
            author: Some(PrincipalId::Object(s.seller)),
            payer: Some(PrincipalId::Organization(s.vendor.org)),
            representative: None,
            facilitator: None,
        },
        deterministic: true,
        deliverables: Vec::new(),
    }
}

/// [`SOLD_GENERIC`], filled in with the market's own names and money render.
#[allow(clippy::too_many_arguments)]
fn sold_generic(
    ctx: &Ctx<'_>,
    listing: &Listing,
    vendor: &Vendor,
    instrument: &InstrumentId,
    quantity: u64,
    gross: Amount,
    net: Amount,
    cuts: &[crate::record::ProceedsCut],
) -> String {
    let name = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&listing.catalog).map(|e| e.name.to_string()))
        .unwrap_or_else(|| listing.catalog.code.to_string());
    let money = |a: Amount| crate::commands::money(ctx.w, Some(instrument), a);
    let cut_text: String = cuts
        .iter()
        .map(|c| format!("; {} {}", c.label, money(c.amount)))
        .collect();
    SOLD_GENERIC
        .replace("{qty}", &quantity.to_string())
        .replace("{name}", &name)
        .replace("{shop}", &vendor.name)
        .replace("{price}", &money(listing.price))
        .replace("{gross}", &money(gross))
        .replace("{cuts}", &cut_text)
        .replace("{net}", &money(net))
}

/// **Ask again for a sale-into-a-bid's hand-over** (doc 18 §4.3, D-18-26; the
/// `restage_draw` precedent of D-18-32; cut IC10). Public because IC11's executor
/// verifies an in-flight sale after a restart through it.
///
/// - `Ok(None)` — not a re-stageable sale: not the actor's, not activated, not a
///   Deliver, settled, no contract, or the contract not `Active`; the caller falls
///   through to its existing answers;
/// - `Err(DELIVER_LANDED)` — (cut IC14 row 3) the leg already landed: its marker is in
///   inventory's applied family, read exactly as the over-sell guard
///   (`delivery_pending`) reads it, **before** the cutoff, so a leg that landed in the
///   last window before `draw_by` is never told "Nothing moved";
/// - `Err(DELIVER_UNCONFIRMED)` — (cut IC14) the marker cannot be read (no lot supply,
///   no marker, an unreadable family): the re-stage cannot know, and says so;
/// - `Err(DELIVER_TOO_LATE)` — no landing, and past the cutoff `draw_by` (real clock):
///   the leg would not be offered at drain;
/// - `Err(<the undeliverable line>)` — the leg could not be staged;
/// - `Ok(Some(contract))` — staged through `paid_leg`: the same contract key and
///   byte-identical parameters as the accept-time leg, so a re-stage after a landing
///   replays as a HIT and moves nothing (the marker is also `transfer_marked`'s replay
///   guard), and one after a refused prepare is prepared again.
///
/// # Errors
///
/// [`MarketError`]; render it with [`crate::refuse_market`].
pub fn restage_deliver(ctx: &mut Ctx<'_>, id: QuoteId) -> Result<Option<ContractId>, MarketError> {
    let Some(q) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.quote(id).cloned())
    else {
        return Ok(None);
    };
    if q.seller != PrincipalId::Object(ctx.actor) || !q.activated || q.settled.is_some() {
        return Ok(None);
    }
    let (Some(lot), Some(contract)) = (q.lot.clone(), q.contract) else {
        return Ok(None);
    };
    let (PaidKind::Deliver, Some(draw_by)) = (lot.kind, lot.draw_by) else {
        return Ok(None);
    };
    let active = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().contract(contract).map(|c| c.state))
        .is_some_and(|state| matches!(state, ContractState::Active));
    if !active {
        return Ok(None);
    }
    // Doc 18 cut IC14 row 3 (lead ruling L3): the marker first, exactly as
    // `delivery_pending` reads it, so a landed leg answers that the goods are in.
    let landed = match (lot.marker.as_ref(), crate::module::lot_supply(ctx.w)) {
        (Some(marker), Some(families)) => ctx.action_state(&families.applied, marker),
        _ => return Err(MarketError::bad(DELIVER_UNCONFIRMED)),
    };
    match landed {
        Ok(Some(_)) => return Err(MarketError::bad(DELIVER_LANDED)),
        Err(_) => return Err(MarketError::bad(DELIVER_UNCONFIRMED)),
        Ok(None) => {}
    }
    if ctx.w.now() > draw_by {
        return Err(MarketError::bad(DELIVER_TOO_LATE));
    }
    if !crate::lot::stage(ctx, &paid_leg(&q, &lot, contract)) {
        return Err(MarketError::bad(UNDELIVERABLE));
    }
    Ok(Some(contract))
}

// ---- the settlement ear -----------------------------------------------------

/// What one contract transition does to the market.
///
/// Called from the market's own [`EconomyObserver`](mush_economy::EconomyObserver)
/// and from nowhere else. **Exhaustive and panic-free**: it runs post-commit, in the
/// world task, on the economy's own emission path.
pub(crate) fn on_contract_changed(
    ctx: &mut Ctx<'_>,
    contract: ContractId,
    kind: mush_economy::ContractStateKind,
) {
    use mush_economy::ContractStateKind as K;
    let Some(quote_id) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.quote_of_contract(contract))
    else {
        return;
    };
    let Some(q) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.quote(quote_id).cloned())
    else {
        return;
    };
    match kind {
        K::Settled => {
            // **Idempotent** (independent verify fold, R9b): a second `Settled`
            // emission used to re-stamp the row — moving it inside or outside the
            // seven-day window — and hand the goods over a second time. It was
            // protected only by `put_contract` emitting on a state *change* and by
            // `republish_settlements`' `is_public()` filter, i.e. by a market purchase
            // being `Parties`, and not by the code that does it.
            if q.settled.is_some() {
                return;
            }
            // The trade-tape row's **authority** is the economy's own contract state,
            // read here and re-read by every projection: the row records the
            // transaction the settlement landed on, and a projection counts it only
            // while the journal still agrees.
            let settled = ctx
                .w
                .module_state::<EconomyStore>()
                .and_then(|s| s.snapshot().contract(contract).map(|c| c.state));
            let Some(ContractState::Settled { transaction }) = settled else {
                return;
            };
            stamp_settlement(ctx, &q, transaction);
            crate::module::deliver(ctx, &q, contract);
            // Doc 18 cut IC10 (lead ruling (e)): one sale into a bid settled — the
            // proceeds policy is told exactly once, behind the `q.settled` guard above.
            // A settlement healed at boot is folded on the market's first sweep instead
            // (doc 18 cut IC14 row 9: `boot::fold_healed_sales`), since the game's store
            // is not loaded when the market boots.
            settled_sale(ctx, &q, contract);
        }
        K::Cancelled | K::Expired => {
            // Doc 18 cut IC7: an activated Draw quote whose mirror has not seen its goods
            // move can only be released after reading its durable receipt, and this ear
            // runs on the emitting module's context (the economy's sweep, for an expiry),
            // where `Ctx::action_state`'s owner scope refuses the market's own family. So
            // it is left exactly as it is, and the draw mirror on the market's own sweep
            // (`boot::reconcile_draws`) releases it within one lap, reading the receipt
            // where the read is the market's to make.
            //
            // Doc 18 cut IC10: an activated, unsettled sale into a bid is deferred the
            // same way — whether its goods moved is its marker in inventory's applied
            // family, read only from the market's own contexts.
            if awaits_receipt_read(&q) {
                return;
            }
            // The goods come back, the maker's window gets its budget back, and the
            // quote is forgotten: a closed record is deleted, so a quote/cancel loop
            // mints no permanent documents.
            release_quote(ctx, &q);
        }
        K::Draft | K::Offered | K::Active | K::ReadyToSettle | K::Disputed => {}
    }
}

/// Stamp the trade-tape row on a quote whose contract really settled.
///
/// `Settlement.at` is the **transaction's own `posted_at`**, never `now` (independent
/// verify fold, R9b): a settlement healed at boot and stamped here would otherwise
/// carry the boot's clock, which moves the row inside the projection's window and makes
/// the published tape a function of when the server restarted.
pub(crate) fn stamp_settlement(
    ctx: &mut Ctx<'_>,
    q: &Quote,
    transaction: mush_economy::TransactionId,
) {
    let at = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.transaction(transaction).map(|t| t.posted_at))
        .unwrap_or_else(|| ctx.w.now());
    put_quote(
        ctx,
        Quote {
            settled: Some(Settlement { transaction, at }),
            revision: q.revision.saturating_add(1),
            ..q.clone()
        },
    );
}

/// Undo one quote's activation: the goods return, the maker's window gets its budget
/// back, and the row is forgotten.
///
/// `del_quote` is the guard that makes it **idempotent** (independent verify fold, R2):
/// a boot reconcile that runs twice cannot double-restore, because the second pass has
/// no quote to key on. A quote whose deal never came into force gives nothing back,
/// because nothing was taken (R9).
///
/// **The no-relist guard** (doc 18 cut IC7, F1 defence in depth). `on_contract_changed`
/// and `reconcile_contracts` both unwind through here, so this is the one place a
/// Draw's listing could be restored over goods that already left. For an activated
/// Draw quote it first decides whether the goods moved — the mirror (`drawn_at`), else
/// the durable receipt; an unreadable receipt counts as moved, because under-listing is
/// safe and phantom stock is not — and when they moved it skips the restore, gives the
/// budgets back, forgets the row, and pages once. With the cutoff this is unreachable in
/// ordinary running; it exists for a mirror starved past the margin. The receipt read is
/// owner-scoped, so an undrawn Draw is released only from a context the market owns (its
/// sweep's mirror arm and its boot reconcile); the settlement ear defers it
/// ([`awaits_receipt_read`]).
pub(crate) fn release_quote(ctx: &mut Ctx<'_>, q: &Quote) {
    if q.activated {
        let moved = goods_moved(ctx, q);
        if moved == GoodsMoved::No {
            restore_quantity(ctx, q);
        } else if q.lot.as_ref().is_some_and(|l| l.kind == PaidKind::Deliver) {
            // Doc 18 cut IC10: a sale into a bid whose goods reached the house while its
            // contract lapsed unpaid — the bid is not restored over goods it received,
            // and staff settle the seller by hand. Cut IC14 row 13: an unconfirmed
            // hand-over says so (still not restored: under-listing is the safe side,
            // lead ruling L5).
            deliver_lapsed_page(ctx, q, moved);
        } else {
            let contract = q.contract.unwrap_or(ContractId(0));
            if moved == GoodsMoved::Unknown {
                tracing::error!(
                    quote = q.id.0,
                    contract = contract.0,
                    "market: a Draw's receipt could not be read and its contract lapsed \
                     unpaid; the listing is not restored"
                );
            } else {
                tracing::error!(
                    quote = q.id.0,
                    contract = contract.0,
                    "market: a Draw's goods moved and its contract lapsed unpaid; the listing \
                     is not restored"
                );
            }
            // An activated quote always names its contract, so the page always has its
            // subject; the arm without one only logs.
            if let Some(contract) = q.contract {
                mush_economy::open_escalation(
                    ctx,
                    mush_economy::EscalationKind::BootReconciliation,
                    mush_economy::EscalationSubject::Contract(contract),
                    lapsed_page_text(PaidKind::Draw, moved, q.id, contract),
                    vec![
                        format!("@contract/show #{contract}"),
                        format!(
                            "@market/catalog show {}:{}",
                            q.catalog.ruleset, q.catalog.code
                        ),
                    ],
                );
            }
        }
        charge_budgets(ctx, q, -1);
    }
    del_quote(ctx, q.id);
}

/// Whether releasing `q` needs its durable draw receipt read (doc 18 cut IC7): an
/// activated Draw quote whose mirror (`drawn_at`) has not yet seen its goods move. Such a
/// quote is released only where the market's own typed state may be read — the mirror arm
/// of its sweep and its boot reconcile — never from the settlement ear.
///
/// Doc 18 cut IC10: an activated, unsettled sale into a bid (`PaidKind::Deliver`) too —
/// its marker is read through `Ctx::action_state` on the market's own contexts only.
pub(crate) fn awaits_receipt_read(q: &Quote) -> bool {
    q.activated
        && match q.lot.as_ref().map(|l| l.kind) {
            Some(PaidKind::Draw) => q.drawn_at.is_none(),
            Some(PaidKind::Deliver) => q.settled.is_none(),
            Some(PaidKind::Hold) | None => false,
        }
}

/// Tell the proceeds policy one sale into a bid settled (doc 18 cut IC10). A no-op for
/// every other quote, and with no policy wired.
pub(crate) fn settled_sale(ctx: &mut Ctx<'_>, q: &Quote, contract: ContractId) {
    let Some(lot) = q.lot.as_ref().filter(|l| l.kind == PaidKind::Deliver) else {
        return;
    };
    let Some(policy) = crate::module::proceeds_policy(ctx.w) else {
        return;
    };
    // Doc 18 cut IC14 row 8 (lead ruling L7): an overflowing gross folds nothing and
    // logs — never a saturated figure into the enrolment's counters.
    let Some(gross) = sale_gross(q) else {
        tracing::error!(
            quote = q.id.0,
            contract = contract.0,
            "market: a settled sale's gross overflows; the proceeds policy is not told"
        );
        return;
    };
    policy.settled(
        ctx,
        &crate::module::SettledSale {
            quote: q.id,
            contract,
            seller: &q.seller,
            source: lot.source.0,
            commodity: &lot.commodity.0,
            quantity: q.quantity,
            price: q.price,
            gross,
            cuts: &q.cuts,
            basis: q.basis,
        },
    );
}

/// A sale's gross, `price × quantity` in minor units, or `None` when it does not fit
/// (doc 18 cut IC14 row 8): checked, never saturated.
fn sale_gross(q: &Quote) -> Option<Amount> {
    gross_of(q.quantity, q.price)
}

/// `price × quantity`, checked ([`sale_gross`]'s arithmetic, unit-tested without a
/// whole `Quote`).
fn gross_of(quantity: u64, price: Amount) -> Option<Amount> {
    i64::try_from(quantity)
        .ok()
        .and_then(|n| price.minor().checked_mul(n))
        .map(Amount::from_minor)
}

/// The page a sale into a bid raises when its goods reached the house and its contract
/// lapsed unpaid (doc 18 cut IC10): the bid is not restored and the seller was not paid.
fn deliver_lapsed_page(ctx: &mut Ctx<'_>, q: &Quote, moved: GoodsMoved) {
    let contract = q.contract.unwrap_or(ContractId(0));
    if moved == GoodsMoved::Unknown {
        tracing::error!(
            quote = q.id.0,
            contract = contract.0,
            "market: a sale's hand-over could not be confirmed and its contract lapsed unpaid; \
             the bid is not restored"
        );
    } else {
        tracing::error!(
            quote = q.id.0,
            contract = contract.0,
            "market: a sale's goods reached the house and its contract lapsed unpaid; the bid \
             is not restored"
        );
    }
    let Some(contract) = q.contract else {
        return;
    };
    let destination = q.lot.as_ref().map_or(0, |l| l.destination.vault().0);
    mush_economy::open_escalation(
        ctx,
        mush_economy::EscalationKind::BootReconciliation,
        mush_economy::EscalationSubject::Contract(contract),
        lapsed_page_text(PaidKind::Deliver, moved, q.id, contract),
        vec![
            format!("@contract/show #{contract}"),
            format!("@inv/show {destination}"),
        ],
    );
}

/// **Whether an activated quote's goods moved** (doc 18 cut IC7; a three-way answer
/// since cut IC14 row 13, named so to avoid `mush_inventory::Moved`). `Unknown` — no
/// lot supply, no marker, no contract, an unreadable family or receipt, or a sale's
/// marker absent past inventory's retention horizon (cut IC14 row 1) — is still
/// **treated as moved** by [`release_quote`] (under-listing is safe; phantom stock is
/// not, lead ruling L5); only the page's words differ, so a page never claims goods
/// moved when it could not tell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GoodsMoved {
    /// The mirror, the receipt or the marker says the goods moved.
    Yes,
    /// They did not (and every quote that is neither a Draw nor a Deliver).
    No,
    /// It could not be read, or a sale's marker is absent past the retention horizon
    /// (cut IC14 row 1); treated as moved.
    Unknown,
}

/// Whether an activated quote's goods **left the listing for good** (doc 18 cut IC7):
/// [`GoodsMoved::Yes`] for a Draw quote whose mirror or durable receipt says it moved,
/// or a sale into a bid whose marker is written; [`GoodsMoved::Unknown`] when that
/// cannot be read, or when a sale's marker is absent once its deal is older than
/// [`mush_inventory::MARKER_RETENTION_SECS`] (absence no longer tells: the marker may
/// have been retired; cut IC14 row 1) — fail closed, treated as moved;
/// [`GoodsMoved::No`] otherwise, and for every other quote, which restores exactly as
/// before IC7.
fn goods_moved(ctx: &Ctx<'_>, q: &Quote) -> GoodsMoved {
    // Doc 18 cut IC10: a sale into a bid moved when its marker is in inventory's applied
    // family; an unreadable family, no lot supply or no marker cannot tell.
    if let Some(lot) = q.lot.as_ref().filter(|l| l.kind == PaidKind::Deliver) {
        let (Some(marker), Some(families)) =
            (lot.marker.as_ref(), crate::module::lot_supply(ctx.w))
        else {
            return GoodsMoved::Unknown;
        };
        return match ctx.action_state(&families.applied, marker) {
            Ok(Some(_)) => GoodsMoved::Yes,
            // Doc 18 cut IC14 row 1: an absent marker means "never written" only inside
            // inventory's retention horizon. Past it the marker may have been retired
            // from its source's ring (a sale disputed for days, its hold moving on), so
            // absence no longer tells: unconfirmed, paged, never restored (L5). The
            // marker's ring stamp is its offer's real second, never before
            // `accepted_at`, so no marker this sale wrote is retired before this.
            Ok(None) if marker_horizon_passed(ctx.w.now(), lot.accepted_at) => GoodsMoved::Unknown,
            Ok(None) => GoodsMoved::No,
            Err(_) => GoodsMoved::Unknown,
        };
    }
    if !q.lot.as_ref().is_some_and(|l| l.kind == PaidKind::Draw) {
        return GoodsMoved::No;
    }
    if q.drawn_at.is_some() {
        return GoodsMoved::Yes;
    }
    let Some(contract) = q.contract else {
        // An activated quote always names its contract (the link is written before
        // either acceptance); one that does not cannot be checked.
        return GoodsMoved::Unknown;
    };
    match crate::lot::draw_receipt(ctx, contract) {
        Ok(Some(_)) => GoodsMoved::Yes,
        Ok(None) => GoodsMoved::No,
        Err(_) => GoodsMoved::Unknown,
    }
}

/// **Whether a sale's marker may already have been retired** (doc 18 cut IC14 row 1):
/// `now` (real seconds) is at or past `accepted_at` +
/// [`mush_inventory::MARKER_RETENTION_SECS`]. Inventory retires a marker only once its
/// ring entry — stamped with its offer's real second, never earlier than the deal's
/// `accepted_at` — is that old, so before this an absent marker was never written.
fn marker_horizon_passed(now: i64, accepted_at: i64) -> bool {
    now >= accepted_at.saturating_add(mush_inventory::MARKER_RETENTION_SECS)
}

/// The staff page a lapsed, activated Draw or Deliver quote opens (doc 18 cut IC7 /
/// IC10; worded by what is known since cut IC14 row 13): the `Yes` texts are the IC7
/// and IC10 ones unchanged; `Unknown` says the movement is not confirmed. A `No` never
/// pages (the listing is restored), and renders as `Yes` would if asked.
fn lapsed_page_text(
    kind: PaidKind,
    moved: GoodsMoved,
    quote: QuoteId,
    contract: ContractId,
) -> String {
    match (kind, moved) {
        (PaidKind::Deliver, GoodsMoved::Unknown) => format!(
            "market quote #{quote}'s hand-over could not be confirmed; contract #{contract} \
             lapsed unpaid and the bid was not restored. Check whether the goods reached the \
             house before settling the seller."
        ),
        (PaidKind::Deliver, GoodsMoved::Yes | GoodsMoved::No) => format!(
            "market sale quote #{quote}'s goods reached the house but contract #{contract} \
             lapsed unpaid, so the bid was not restored. The seller was not paid; settle it by \
             hand."
        ),
        (PaidKind::Draw | PaidKind::Hold, GoodsMoved::Unknown) => format!(
            "market quote #{quote}'s draw receipt could not be read, so whether its goods were \
             drawn is not confirmed; contract #{contract} lapsed unpaid and the listing was not \
             restored. The payment came back."
        ),
        (PaidKind::Draw | PaidKind::Hold, GoodsMoved::Yes | GoodsMoved::No) => format!(
            "market quote #{quote}'s goods were drawn but contract #{contract} lapsed unpaid, so \
             the listing was not restored. The buyer holds the goods and the payment came back."
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Doc 18 cut IC14 row 13: the lapsed-quote page says "could not confirm" when it
    /// could not, for both kinds; the confirmed texts are the IC7 / IC10 ones.
    #[test]
    fn ic14_the_lapsed_page_says_what_is_known() {
        let (q, c) = (QuoteId(7), ContractId(9));
        assert_eq!(
            lapsed_page_text(PaidKind::Draw, GoodsMoved::Yes, q, c),
            "market quote #7's goods were drawn but contract #9 lapsed unpaid, so the listing \
             was not restored. The buyer holds the goods and the payment came back."
        );
        assert_eq!(
            lapsed_page_text(PaidKind::Draw, GoodsMoved::Unknown, q, c),
            "market quote #7's draw receipt could not be read, so whether its goods were drawn \
             is not confirmed; contract #9 lapsed unpaid and the listing was not restored. The \
             payment came back."
        );
        assert_eq!(
            lapsed_page_text(PaidKind::Deliver, GoodsMoved::Yes, q, c),
            "market sale quote #7's goods reached the house but contract #9 lapsed unpaid, so \
             the bid was not restored. The seller was not paid; settle it by hand."
        );
        assert_eq!(
            lapsed_page_text(PaidKind::Deliver, GoodsMoved::Unknown, q, c),
            "market quote #7's hand-over could not be confirmed; contract #9 lapsed unpaid and \
             the bid was not restored. Check whether the goods reached the house before \
             settling the seller."
        );
    }

    /// Doc 18 cut IC14 row 1 (review F1): absence of a sale's marker tells only inside
    /// inventory's retention horizon, measured from the deal's `accepted_at`.
    #[test]
    fn ic14_the_marker_horizon_is_the_retention_from_acceptance() {
        let accepted = 10_000;
        let horizon = accepted + mush_inventory::MARKER_RETENTION_SECS;
        assert!(!marker_horizon_passed(accepted, accepted));
        assert!(!marker_horizon_passed(horizon - 1, accepted));
        assert!(marker_horizon_passed(horizon, accepted));
        assert!(
            marker_horizon_passed(i64::MAX, i64::MAX),
            "saturating, never wraps"
        );
    }

    /// Doc 18 cut IC14 row 8: a gross that does not fit is `None`, never saturated.
    #[test]
    fn ic14_an_overflowing_gross_is_none() {
        let price = Amount::from_minor(2_800);
        assert_eq!(gross_of(u64::MAX, price), None);
        assert_eq!(
            gross_of(u64::MAX / 2, price),
            None,
            "fits i64, overflows the product"
        );
        assert_eq!(gross_of(20, price), Some(Amount::from_minor(56_000)));
    }
}

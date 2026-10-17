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
        Ok(AcceptOutcome::NeedsApproval(_)) | Err(_) => {
            let _ = mush_economy::cancel_contract(ctx, contract);
            return Err(MarketError::bad(
                "That purchase needs an approval your account rule has not granted yet. Nothing \
                 was agreed.",
            ));
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
fn paid_leg(q: &Quote, lot: &PaidLot, contract: ContractId) -> crate::lot::LotLeg {
    crate::lot::LotLeg {
        op: match lot.kind {
            PaidKind::Hold => crate::lot::LotOp::Consume,
            PaidKind::Draw => crate::lot::LotOp::Draw,
        },
        listing: q.listing,
        endpoint: lot.destination.0,
        source: lot.source.0,
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
    if left == 0 {
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
        }
        K::Cancelled | K::Expired => {
            // Doc 18 cut IC7: an activated Draw quote whose mirror has not seen its goods
            // move can only be released after reading its durable receipt, and this ear
            // runs on the emitting module's context (the economy's sweep, for an expiry),
            // where `Ctx::action_state`'s owner scope refuses the market's own family. So
            // it is left exactly as it is, and the draw mirror on the market's own sweep
            // (`boot::reconcile_draws`) releases it within one lap, reading the receipt
            // where the read is the market's to make.
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
        if !goods_moved(ctx, q) {
            restore_quantity(ctx, q);
        } else {
            let contract = q.contract.unwrap_or(ContractId(0));
            tracing::error!(
                quote = q.id.0,
                contract = contract.0,
                "market: a Draw's goods moved and its contract lapsed unpaid; the listing is \
                 not restored"
            );
            // An activated quote always names its contract, so the page always has its
            // subject; the arm without one only logs.
            if let Some(contract) = q.contract {
                mush_economy::open_escalation(
                    ctx,
                    mush_economy::EscalationKind::BootReconciliation,
                    mush_economy::EscalationSubject::Contract(contract),
                    format!(
                        "market quote #{}'s goods were drawn but contract #{contract} lapsed \
                         unpaid, so the listing was not restored. The buyer holds the goods and \
                         the payment came back.",
                        q.id
                    ),
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
pub(crate) fn awaits_receipt_read(q: &Quote) -> bool {
    q.activated && q.drawn_at.is_none() && q.lot.as_ref().is_some_and(|l| l.kind == PaidKind::Draw)
}

/// Whether an activated quote's goods **left the listing for good** (doc 18 cut IC7):
/// `true` only for a Draw quote whose mirror or durable receipt says it moved — or whose
/// receipt cannot be read, or which names no contract to read it by, both of which
/// fail closed (under-listing is safe; phantom stock is not). Every other quote is
/// `false`, and restores exactly as before IC7.
fn goods_moved(ctx: &Ctx<'_>, q: &Quote) -> bool {
    if !q.lot.as_ref().is_some_and(|l| l.kind == PaidKind::Draw) {
        return false;
    }
    if q.drawn_at.is_some() {
        return true;
    }
    let Some(contract) = q.contract else {
        // An activated quote always names its contract (the link is written before
        // either acceptance); one that does not cannot be checked.
        return true;
    };
    !matches!(crate::lot::draw_receipt(ctx, contract), Ok(None))
}

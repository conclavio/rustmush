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
use crate::record::{
    AvailabilityRule, Listing, MAX_OPEN_QUOTES_PER_BUYER, MAX_QUOTES, ObservedRevisions, Quote,
    Representation, Settlement, Supply, Vendor, declared_hash,
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
    if let Supply::Market { source } = listing.source
        && store.source(source).is_none_or(|s| s.halted)
    {
        return false;
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
    let Some(vendor) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor(listing.vendor).cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    };
    if !vendor.is_trading() {
        return Err(MarketError::bad(STALE_QUOTE));
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
    let unit = price_for(&listing, ctx.w.module_state::<MarketStore>());
    let total = Amount::from_minor(
        unit.minor()
            .saturating_mul(i64::try_from(quantity).unwrap_or(i64::MAX)),
    );
    // ---- B8: every budget, before the quote is issued ---------------------
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
        let baseline = entry.baseline.unwrap_or(Amount::ZERO).minor();
        // The loss a sale at this price books against the reference price. A maker
        // selling *above* baseline books no loss; a maker selling below books the
        // difference, and its loss budget is what stops it selling the world cheaply.
        let loss = Amount::from_minor(
            baseline
                .saturating_sub(unit.minor())
                .max(0)
                .saturating_mul(i64::try_from(quantity).unwrap_or(i64::MAX)),
        );
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
    if ctx
        .w
        .module_state::<MarketStore>()
        .is_some_and(|s| s.quotes().count() >= MAX_QUOTES)
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
                Supply::Stock { .. } | Supply::Procurement { .. } => None,
            },
            price: unit,
            quantity,
            definition_version: entry.definition_version,
            observed: ObservedRevisions {
                definition_version: entry.definition_version,
                listing: listing.revision,
                artefact: artefact_stamp(entry.representation, &listing.catalog),
            },
            expires: EconomicDeadline {
                clock: EconomicClock::RealUtc,
                at: now.saturating_add(ttl),
            },
            availability,
            contract: None,
            settled: None,
            revision: 1,
        },
    );
    persist_meta(ctx);
    Ok(id)
}

/// The one refusal every exhausted budget gets. It names the budget, because a
/// maker's budgets are **declared** (doc 10 7A) and telling a buyer which one ran out
/// discloses nothing a `@market/source show` would not.
fn maker_refusal(which: &str) -> String {
    if which == "halted" {
        return "That market maker is not quoting.".to_string();
    }
    format!("That market maker has run out of {which} for this window and is not quoting.")
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

/// A listing's unit price. Stock and procurement ask their own price; a maker's is
/// the listing price moved by its declared spread, which is a **trading policy** and
/// never a substitute for funding (doc 10 7A).
fn price_for(l: &Listing, store: Option<&MarketStore>) -> Amount {
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
    let Some(representation) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&q.catalog).map(|e| e.representation))
    else {
        return Err(MarketError::NoStanding(NO_SUCH_QUOTE));
    };
    let Some(instrument) = crate::api::account_instrument(ctx, vendor.account) else {
        return Err(MarketError::NoStanding(NO_SUCH_QUOTE));
    };
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
        ctx.w.now(),
    );
    let contract = mush_economy::offer_contract(ctx, spec).map_err(MarketError::Contract)?;
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
    // `with_party_caps` (G8). That builder grants no money authority whatever: the
    // accept still runs `accept_core`, still places the vendor's escrow (there is
    // none here — it owes no consideration) under the org account's own authority,
    // and still refuses when the org cannot stand behind the deal.
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
    charge_budgets(ctx, &q, listing.catalog.clone(), 1);
    q.contract = Some(contract);
    q.revision = q.revision.saturating_add(1);
    put_quote(ctx, q);
    Ok(contract)
}

/// Take `quantity` off a listing, deleting it when it is exhausted.
fn commit_quantity(ctx: &mut Ctx<'_>, listing: &Listing, quantity: u64) {
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
fn charge_budgets(ctx: &mut Ctx<'_>, q: &Quote, catalog: mush_economy::CatalogRef, sign: i64) {
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
    let baseline = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&catalog).and_then(|e| e.baseline))
        .map_or(0, |b| b.minor());
    let qty = i64::try_from(q.quantity).unwrap_or(i64::MAX);
    let total = q.total().minor();
    let loss = baseline
        .saturating_sub(q.price.minor())
        .max(0)
        .saturating_mul(qty);
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
) -> ContractSpec {
    let total = q.total();
    // `Procurement` carries its **lead time as a `Before` deadline** (§5.4): the
    // delivery must land inside it, and past it the row stops holding and the
    // contract's own expiry refunds. Everything else is an immediate sale whose one
    // condition is the deal window itself.
    let (conditions, deliverables) = match (&listing.source, representation, listing.object) {
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
            at: now.saturating_add(DEAL_WINDOW_SECS),
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
            let now = ctx.w.now();
            put_quote(
                ctx,
                Quote {
                    settled: Some(Settlement {
                        transaction,
                        at: now,
                    }),
                    revision: q.revision.saturating_add(1),
                    ..q.clone()
                },
            );
            crate::module::deliver(ctx, &q, contract);
        }
        K::Cancelled | K::Expired => {
            // The goods come back, the maker's window gets its budget back, and the
            // quote is forgotten: a closed record is deleted, so a quote/cancel loop
            // mints no permanent documents.
            restore_quantity(ctx, &q);
            charge_budgets(ctx, &q, q.catalog.clone(), -1);
            del_quote(ctx, quote_id);
        }
        K::Draft | K::Offered | K::Active | K::ReadyToSettle | K::Disputed => {}
    }
}

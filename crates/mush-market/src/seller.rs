//! **The personal seller** (doc 48 cut EX.1a; Q-48-2, -5, -44, -49, -50, -54).
//!
//! Before this cut only a staff-registered vendor organization could sell on the market.
//! Here any principal with an account and an endpoint it may trade sells, and none of it
//! is a vendor row (Q-48-44):
//!
//! - **an ask** is one exact lot (Q-48-49: split it to size with `@inv/split`), held for
//!   the listing by the market's lot action exactly as a vendor's lot listing is. The lot
//!   hold has no clock of its own, so the listing stands for its own window and is not cut
//!   to the seven-IC-day reservation (Q-48-54's first point);
//! - **a bid** escrows `price × quantity` at posting: a reservation hold on the bidder's
//!   own account, drawn as the bid fills and released at withdrawal or expiry (Q-48-50);
//! - **a private offer** names its one counterparty; it is hidden from everyone else and
//!   refused to anyone else at quote and at accept (Q-48-5);
//! - the caps: [`crate::MAX_LISTINGS_PER_SELLER`] a seller, inside the store's
//!   [`crate::MAX_LISTINGS`].
//!
//! **Standing acceptance** (Q-48-50). A listing is its poster's acceptance, in advance, of
//! any sale within its terms. [`ListingAcceptance`] is the economy's
//! `StandingAcceptance` seam: it says yes for a sale contract only while the listing
//! stands, at the listing's id, revision, price and quantity, for the listing's own seller,
//! and for a bid only against the bid's own escrow. A withdrawal deletes the listing, so it
//! revokes the acceptance. The market then accepts for the absent seller through
//! `mush_economy::accept_contract_standing`, under the seller's own debit authority.
//!
//! Every player-facing sentence this cut adds is a `pub const` here, so the bot's sentence
//! table can hold it to the game's text.

use mush_core::{Ctx, ObjId, World};
use mush_economy::{
    AccountId, Amount, CatalogRef, EconomicClock, EconomicContract, EconomicDeadline, EconomyStore,
    HoldId, InstrumentId,
};
use mush_org::PrincipalId;

use crate::api::{MarketError, NO_SUCH_LISTING, principal_account};
use crate::ids::{ListingId, MarketId};
use crate::record::{
    BID_ESCROW_KIND, CatalogEntry, Listing, LotCommodity, LotEndpoint, MAX_LISTINGS,
    MAX_LISTINGS_PER_SELLER, MAX_QUANTITY, MarketVisibility, NO_VENDOR, Representation, Supply,
};
use crate::store::{MarketStore, del_listing, is_live_object, persist_meta, put_listing};

// ---- the sentences -----------------------------------------------------------

/// An ask posted: `{listing}`, `{qty}`, `{name}`, `{price}`, `{to}` (empty, or
/// [`FOR_ONLY`] filled).
pub const ASK_POSTED: &str = "Listed for sale as #{listing}: {qty} {name} at {price} each{to}.";
/// The second line after an ask or a bid is posted: `{listing}`.
pub const POSTED_AWAY: &str =
    "It trades while you are away. +market/withdraw {listing} takes it down.";
/// A bid posted: `{listing}`, `{qty}`, `{name}`, `{price}`, `{total}`, `{to}`.
pub const BID_POSTED: &str =
    "Bid #{listing}: {qty} {name} at {price} each{to}; {total} is held from your account.";
/// A private offer's clause: `{who}`.
pub const FOR_ONLY: &str = " (a private offer to {who})";
/// The listing line's mark on a private offer (seen only by its seller, its counterparty
/// and staff).
pub const PRIVATE_MARK: &str = " (private offer)";
/// `+market/show`'s supply line for a personal bid.
pub const BID_SUPPLY: &str = "a standing bid, escrowed; the bidder buys this";
/// A listing withdrawn: `{listing}`.
pub const WITHDRAWN: &str = "Withdrew #{listing}; its goods are yours to use again.";
/// A bid withdrawn: `{listing}`.
pub const BID_WITHDRAWN: &str = "Withdrew bid #{listing}; what it still held is yours again.";
/// The heading of `+market/mine`.
pub const MINE_HEAD: &str = "Your listings:";
/// `+market/mine` with nothing listed.
pub const MINE_NONE: &str = "You have nothing listed.";
/// `+market/mine`'s count line: `{n}`, `{cap}`.
pub const MINE_COUNT: &str = "You hold {n} of the {cap} listings a seller may.";
/// The existence-hiding answer for a lot the actor may not take, or that does not exist.
pub const NO_SUCH_LOT: &str = "No such lot.";
/// The existence-hiding answer for a vault the actor may not take from.
pub const NO_SUCH_VAULT: &str = "No such vault.";
/// The lot is held for something else (another listing, an order).
pub const LOT_HELD: &str = "That lot is held for something else, so nothing was listed.";
/// The lot's good is not one the market lists as lots.
pub const NOT_A_LOT_GOOD: &str = "This market does not list that good, so nothing was listed.";
/// A lot larger than a listing sells.
pub const LOT_TOO_BIG: &str =
    "That lot is larger than one listing sells; split it with @inv/split.";
/// The seller's cap: `{cap}`.
pub const SELLER_FULL: &str =
    "You hold {cap} listings, as many as a seller may. Withdraw one first.";
/// No venue named, and the room is in none.
pub const NAME_VENUE: &str = "Name the venue it trades in: market=<venue>.";
/// The venue has no currency yet (no vendor trades there).
pub const NO_CURRENCY: &str = "That venue trades in no currency yet, so nothing was listed there.";
/// The seller has no account in the venue's currency.
pub const NO_ACCOUNT: &str = "You have no account in that venue's currency to trade from.";
/// A private offer named nobody, or the seller.
pub const PRIVATE_NAMES_OTHER: &str = "A private offer names one other player: to=<player>.";
/// A private offer taken up by someone else: refused at quote and at accept.
pub const PRIVATE_TO_OTHER: &str =
    "That offer is made to someone else, so you cannot take it up. Nothing was charged.";
/// The actor's own listing, quoted or sold into: `{listing}`.
pub const OWN_LISTING: &str = "That is your own listing; +market/withdraw {listing} takes it down.";
/// A second bid for one good: `{listing}`.
pub const BID_TWICE: &str = "You already bid for that good (#{listing}); withdraw that bid first.";
/// A bulk bid into a governed hold.
pub const BID_INTO_HOLD: &str = "A bid for bulk goods receives into a vault, not a hold: goods are poured into a hold only \
     from its port's own counters.";
/// The bid's escrow could not be held: `{total}`.
pub const BID_CANNOT_COVER: &str =
    "Your account does not have {total} available to hold for that bid, so nothing was posted.";
/// The bid's escrow needs an approval.
pub const BID_NEEDS_APPROVAL: &str =
    "Holding that bid needs an approval your account rule has not granted yet. Nothing was posted.";
/// The bid's escrow was refused for another reason.
pub const BID_NOT_HELD: &str = "That bid cannot be held right now, so nothing was posted.";
/// A personal bid that can no longer pay for the sale (its escrow gone or short).
pub const BIDDER_NOT_BUYING: &str = "That bid is not buying any more, so nothing was sold.";
/// A personal seller who cannot complete the sale (gone, or no longer standing behind it).
pub const SELLER_GONE: &str = "That seller cannot complete the sale right now. Nothing was agreed.";
/// The `+market/post` grammar.
pub const POST_USAGE: &str = "Usage: +market/post <lot>|#<object> price=<n> [to=<player>] \
     [market=<venue>] [days=<n>] [as=<org>]";
/// `+market/post … as=<org>` naming an organization the actor does not act for (doc 48 cut
/// PL.9). An unknown or hidden organization reads the same.
pub const NOT_FOR_ORG: &str = "You do not act for that organization, so nothing was listed.";
/// The `+market/bid` grammar.
pub const BID_USAGE: &str = "Usage: +market/bid <ruleset>:<code> qty=<n> price=<n> into=<vault> [to=<player>] \
     [market=<venue>] [days=<n>]";
/// The `+market/withdraw` grammar.
pub const WITHDRAW_USAGE: &str = "Usage: +market/withdraw <listing>";
/// A named player who is not found.
pub const NO_SUCH_PLAYER: &str = "No such player.";
/// Doc 48 cut EX.2: a titled object put up for sale: `{name}`, `{listing}`, `{price}`, `{to}`.
pub const ASSET_POSTED: &str = "Put the {name} up for sale as #{listing} at {price}{to}.";
/// Doc 48 cut EX.2: the existence-hiding answer for an object the seller holds no live title to.
pub const NO_SUCH_ASSET: &str = "No such asset.";
/// Doc 48 cut EX.2: an object the market does not list for a person.
pub const NOT_LISTABLE: &str = "That is not something this market lists for you.";
/// Doc 48 cut EX.2: an object already listed or already being sold.
pub const ALREADY_FOR_SALE: &str = "That is already up for sale; one listing or sale at a time.";

/// The default window a personal listing stands for, in real seconds: fourteen real days,
/// the market's default (336 IC days at 24:1).
pub const PERSONAL_LISTING_TTL_SECS: i64 = crate::api::DEFAULT_LISTING_TTL_SECS;

// ---- who sells a listing ---------------------------------------------------------

/// **The account a listing's seller trades from** (doc 48 cut EX.1a): a personal
/// listing's own, else its vendor row's. `None` for a vendor listing whose row is gone.
#[must_use]
pub fn seller_account(store: &MarketStore, listing: &Listing) -> Option<AccountId> {
    match listing.account {
        Some(account) => Some(account),
        None => store.vendor(listing.vendor).map(|v| v.account),
    }
}

/// **Whether a personal seller may still trade** (doc 48 cut EX.1a): a person who is a
/// live object, or an organization that may agree to new trade. `false` for a vendor
/// listing (its vendor row answers that) and for a system principal.
#[must_use]
pub fn personal_trading(w: &World, listing: &Listing) -> bool {
    listing.is_personal()
        && match &listing.seller {
            PrincipalId::Object(who) => is_live_object(w, *who),
            PrincipalId::Organization(org) => crate::api::org_may_trade(w, *org),
            PrincipalId::System(_) => false,
        }
}

/// **The name a listing line gives its seller** (doc 48 cut EX.1a): a vendor's shop
/// sign, a person's name, or an organization's name unless it is hidden. Empty when none
/// is known.
#[must_use]
pub fn seller_name(w: &World, listing: &Listing) -> String {
    if !listing.is_personal() {
        return w
            .module_state::<MarketStore>()
            .and_then(|s| s.vendor(listing.vendor).map(|v| v.name.to_string()))
            .unwrap_or_default();
    }
    match &listing.seller {
        PrincipalId::Object(who) => w.get(*who).map(|o| o.name.to_string()).unwrap_or_default(),
        PrincipalId::Organization(org) if !crate::api::is_hidden_org(w, *org) => w
            .module_state::<mush_org::OrgStore>()
            .and_then(|st| st.state().org(*org).map(|o| o.name.to_string()))
            .unwrap_or_default(),
        PrincipalId::Organization(_) | PrincipalId::System(_) => String::new(),
    }
}

/// Whether `actor` may sell as `seller` (doc 48 cut EX.1a): themself, or an organization
/// they act for that may agree to new trade.
fn may_sell_as(w: &World, actor: ObjId, seller: &PrincipalId) -> bool {
    match seller {
        PrincipalId::Object(who) => *who == actor,
        PrincipalId::Organization(org) => {
            mush_economy::acts_for(w, actor, seller) && crate::api::org_may_trade(w, *org)
        }
        PrincipalId::System(_) => false,
    }
}

/// The lot-represented catalogue entry for a commodity: the first, by slug, whose code is
/// the commodity id.
fn lot_entry(store: &MarketStore, commodity: &mush_core::CanonicalId) -> Option<CatalogEntry> {
    store
        .entries()
        .find(|e| e.representation == Representation::Lot && e.catalog.code == commodity.as_str())
        .cloned()
}

/// The private-offer rule at posting: a live person who is not the seller.
pub(crate) fn check_private(
    w: &World,
    seller: &PrincipalId,
    to: Option<&PrincipalId>,
) -> Result<(), MarketError> {
    match to {
        None => Ok(()),
        Some(PrincipalId::Object(who))
            if PrincipalId::Object(*who) != *seller && is_live_object(w, *who) =>
        {
            Ok(())
        }
        Some(_) => Err(MarketError::bad(PRIVATE_NAMES_OTHER)),
    }
}

/// The two listing caps, asked before any write: the seller's and the store's.
fn check_caps(w: &World, seller: &PrincipalId) -> Result<(), MarketError> {
    let Some(store) = w.module_state::<MarketStore>() else {
        return Err(MarketError::bad("The market is full; try later."));
    };
    // Doc 48e (exchange G2): a seller holds at most 32 *open* listings (Q-48-44). A lapsed one
    // (a sold lot's, kept until its Consume lands; an expired one, kept until its hold is
    // released) is open to nobody, so it does not count; `MAX_LISTINGS` still bounds them all.
    let now = w.now();
    let open = store
        .personal_listings_of(seller)
        .filter(|l| !crate::api::listing_lapsed(l, now))
        .count();
    if open >= MAX_LISTINGS_PER_SELLER {
        return Err(MarketError::bad(
            SELLER_FULL.replace("{cap}", &MAX_LISTINGS_PER_SELLER.to_string()),
        ));
    }
    if store.listings.len() >= MAX_LISTINGS {
        return Err(MarketError::bad("The market is full; try later."));
    }
    Ok(())
}

/// `price × quantity`, checked.
fn total_of(price: Amount, quantity: u64) -> Option<Amount> {
    i64::try_from(quantity)
        .ok()
        .and_then(|q| price.minor().checked_mul(q))
        .map(Amount::from_minor)
}

// ---- an ask: one exact lot -------------------------------------------------------

/// What [`post_ask`] lists (doc 48 cut EX.1a).
#[derive(Clone, Debug)]
pub struct PersonalAsk {
    /// Who sells: the actor, or an organization the actor acts for.
    pub seller: PrincipalId,
    /// The venue.
    pub market: MarketId,
    /// The venue's currency, which the seller is paid in.
    pub instrument: InstrumentId,
    /// The exact lot sold (Q-48-49).
    pub lot: mush_inventory::LotId,
    /// The asking price, per unit.
    pub price: Amount,
    /// A private offer's one buyer.
    pub private_to: Option<PrincipalId>,
    /// How long it stands, in real seconds (clamped to `60..=MAX_LISTING_TTL_SECS`).
    pub ttl_secs: i64,
}

/// **List one exact lot** (doc 48 cut EX.1a; Q-48-2, -49): the seller's own lot, from an
/// endpoint the actor may take from, held for the listing by the market's lot action.
///
/// Refused before any write: a seller the actor may not sell as; a lot that does not
/// exist or sits where the actor may not take from (one answer, [`NO_SUCH_LOT`]); a lot
/// already held; a good the catalogue does not list as lots; a lot over
/// [`MAX_QUANTITY`]; a second live listing on the lot; a non-positive price or one whose
/// total overflows; a private offer naming nobody else; no account in the venue's
/// currency; the venue gate; the seller's and the store's caps. The listing's window is
/// its own, never cut to the inventory's reservation clock: an exact lot's hold has none.
///
/// # Errors
///
/// [`MarketError`]; render it with [`crate::refuse_market`].
pub fn post_ask(ctx: &mut Ctx<'_>, spec: PersonalAsk) -> Result<ListingId, MarketError> {
    let actor = ctx.actor;
    if !may_sell_as(ctx.w, actor, &spec.seller) {
        return Err(MarketError::NoStanding(NO_SUCH_LOT));
    }
    let Some(families) = crate::module::lot_supply(ctx.w) else {
        return Err(MarketError::bad(
            "This server cannot move lot stock right now, so nothing was listed.",
        ));
    };
    let Ok(Some(lot)) = ctx
        .action_state(&families.lots, &spec.lot)
        .map(|l| l.cloned())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_LOT));
    };
    let endpoint = lot.endpoint();
    if !mush_inventory::may_take(ctx.w, actor, endpoint) {
        return Err(MarketError::NoStanding(NO_SUCH_LOT));
    }
    if !matches!(ctx.action_state(&families.lot_holds, &spec.lot), Ok(None)) {
        return Err(MarketError::bad(LOT_HELD));
    }
    let commodity = lot.commodity().clone();
    let Some(entry) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| lot_entry(s, &commodity))
    else {
        return Err(MarketError::bad(NOT_A_LOT_GOOD));
    };
    let quantity = lot.quantity();
    if quantity == 0 || quantity > MAX_QUANTITY {
        return Err(MarketError::bad(LOT_TOO_BIG));
    }
    // The one-live-listing-per-lot rule `list_goods` keeps: no pooled listing on the
    // pool, and no listing naming this lot.
    if ctx.w.module_state::<MarketStore>().is_some_and(|s| {
        s.lot_listings_of(endpoint, &commodity)
            .any(|l| l.source.exact_lot().is_none_or(|held| held == spec.lot))
    }) {
        return Err(MarketError::bad(LOT_HELD));
    }
    if spec.price.minor() <= 0 {
        return Err(MarketError::bad("A listing asks a positive price."));
    }
    if total_of(spec.price, quantity).is_none() {
        return Err(MarketError::bad(
            "That is not a price this market sells at.",
        ));
    }
    check_private(ctx.w, &spec.seller, spec.private_to.as_ref())?;
    let Some(account) = principal_account(ctx, &spec.instrument, &spec.seller) else {
        return Err(MarketError::bad(NO_ACCOUNT));
    };
    if let Some(gate) = crate::module::venue_gate(ctx.w) {
        gate.admit(
            ctx.w,
            actor,
            &spec.market,
            Some(endpoint),
            crate::module::Presence::Here,
        )
        .map_err(|r| MarketError::bad(r.line))?;
    }
    // Doc 48 cut EX.1b (Q-48-56): the proceeds policy may refuse an ask from its source.
    if let Some(policy) = crate::module::proceeds_policy(ctx.w) {
        policy
            .may_list(ctx.w, &spec.seller, endpoint)
            .map_err(|r| MarketError::bad(r.line))?;
    }
    check_caps(ctx.w, &spec.seller)?;
    let ttl = spec.ttl_secs.clamp(60, crate::api::MAX_LISTING_TTL_SECS);
    let now = ctx.w.now();
    let Some(id) = ctx
        .w
        .module_state_mut::<MarketStore>()
        .and_then(MarketStore::alloc_listing)
    else {
        return Err(MarketError::bad("The listing-id allocator is exhausted."));
    };
    // Doc 48 cut EX.4 (Q-48-8): the listing fee, once, the last fallible step before the
    // listing is written (refunded below if its hold cannot be staged).
    let listed = total_of(spec.price, quantity).unwrap_or(Amount::ZERO);
    let fee_market = spec.market.clone();
    let fee =
        crate::fees::charge_listing_fee(ctx, &spec.seller, account, &spec.market, id, listed)?;
    let grade = lot.grade().get();
    put_listing(
        ctx,
        Listing {
            id,
            seller: spec.seller,
            vendor: NO_VENDOR,
            market: spec.market,
            catalog: entry.catalog.clone(),
            definition_version: entry.definition_version,
            source: Supply::Lot {
                endpoint: LotEndpoint(endpoint),
                commodity: LotCommodity(commodity.clone()),
                grade,
                lot: Some(spec.lot),
            },
            object: None,
            price: spec.price,
            quantity,
            visibility: if spec.private_to.is_some() {
                MarketVisibility::Hidden
            } else {
                MarketVisibility::Public
            },
            expires: EconomicDeadline {
                clock: EconomicClock::RealUtc,
                at: now.saturating_add(ttl),
            },
            revision: 1,
            draw: Some(false),
            account: Some(account),
            private_to: spec.private_to,
            escrow: None,
        },
    );
    persist_meta(ctx);
    // The hold, staged after the document that names it (`list_goods`' order).
    let staged = crate::lot::stage(
        ctx,
        &crate::lot::LotLeg {
            op: crate::lot::LotOp::Reserve,
            listing: id,
            endpoint,
            source: endpoint,
            commodity,
            quantity,
            grade,
            destination: None,
            contract: None,
            lot: Some(spec.lot),
        },
    );
    if !staged {
        del_listing(ctx, id);
        if let Some((fee, _)) = fee {
            refund_listing_fee(ctx, &fee_market, account, id, fee);
        }
        return Err(MarketError::bad(
            "This server cannot move lot stock right now, so nothing was listed.",
        ));
    }
    if let Some(store) = ctx.w.module_state_mut::<MarketStore>() {
        let at = store.sweeps;
        store.reserve_staged.insert(id, at);
    }
    Ok(id)
}

/// **Give back a listing fee whose listing was not written** (doc 48 cut EX.4): the operator
/// pays `fee` back to `account` under `market.listing_fee.refund/<listing>`. Logged when refused;
/// the fee then stands as a staff matter.
fn refund_listing_fee(
    ctx: &mut Ctx<'_>,
    market: &MarketId,
    account: AccountId,
    listing: ListingId,
    fee: Amount,
) {
    let Some(venue) = crate::fees::venue_accounts(ctx.w, market) else {
        return;
    };
    let mut plan = mush_economy::EconomyPlan::new(
        mush_economy::IdempotencyKey::new(
            smol_str::SmolStr::new_static("market.listing_fee.refund"),
            smol_str::SmolStr::new(listing.0.to_string()),
        ),
        PrincipalId::Object(mush_core::GOD),
        mush_economy::EconomicCause::Command {
            serial: ctx.w.next_correlation_id(),
        },
        crate::fees::fee_lines(venue.operator, account, fee),
    );
    plan.description = format!("market listing #{listing} fee refunded");
    if let Err(error) = mush_economy::post(ctx, &plan) {
        tracing::error!(listing = listing.0, %error, "market: a listing fee could not be refunded");
    }
}

// ---- doc 48 cut EX.2: one titled object -----------------------------------------

/// What [`post_asset`] lists (doc 48 cut EX.2).
#[derive(Clone, Debug)]
pub struct PersonalAssetAsk {
    /// Who sells: the actor, or an organization the actor acts for.
    pub seller: PrincipalId,
    /// The venue.
    pub market: MarketId,
    /// The venue's currency, which the seller is paid in.
    pub instrument: InstrumentId,
    /// The titled object sold (a hull).
    pub object: ObjId,
    /// The whole price.
    pub price: Amount,
    /// A private offer's one buyer.
    pub private_to: Option<PrincipalId>,
    /// How long it stands, in real seconds (clamped to `60..=MAX_LISTING_TTL_SECS`).
    pub ttl_secs: i64,
}

/// **List one titled object** (doc 48 cut EX.2; Q-48-7, -52): the seller's own live,
/// unfrozen title, admitted by the composition's asset-sale seam as one catalogue entry
/// (a hull: no fitted governed core, an empty hold, no work in progress). The listing names
/// [`crate::NO_VAULT`] and the object; a purchase is the one asset sale
/// ([`crate::asset_sale_spec`]), pinned to the state the quote priced.
///
/// Refused before any write: a seller the actor may not sell as, or an object they hold no
/// live title to (one answer, [`NO_SUCH_ASSET`]); a governed core (it sells only in its own
/// family); no seam, a seam that does not know the object, or no asset entry for it
/// ([`NOT_LISTABLE`]); the seam's own refusal; an object already listed or being sold
/// ([`ALREADY_FOR_SALE`]); a non-positive price; a private offer naming nobody else; no account
/// in the venue's currency; the venue gate; the caps.
///
/// # Errors
///
/// [`MarketError`]; render it with [`crate::refuse_market`].
pub fn post_asset(ctx: &mut Ctx<'_>, spec: PersonalAssetAsk) -> Result<ListingId, MarketError> {
    let actor = ctx.actor;
    if !may_sell_as(ctx.w, actor, &spec.seller) {
        return Err(MarketError::NoStanding(NO_SUCH_ASSET));
    }
    let held = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().asset(spec.object).cloned())
        .is_some_and(|a| {
            a.title_owner == spec.seller
                && a.lifecycle == mush_economy::AssetLifecycle::Active
                && !a.frozen
        });
    if !held || !is_live_object(ctx.w, spec.object) {
        return Err(MarketError::NoStanding(NO_SUCH_ASSET));
    }
    if ctx
        .w
        .module_state::<EconomyStore>()
        .is_some_and(|s| s.is_managed_core(spec.object))
    {
        return Err(MarketError::bad(crate::trade::MANAGED_CORE_SALE));
    }
    let Some(seam) = crate::module::asset_sale(ctx.w) else {
        return Err(MarketError::bad(NOT_LISTABLE));
    };
    let catalog = match seam.admit(ctx, spec.object, &spec.seller) {
        None => return Err(MarketError::bad(NOT_LISTABLE)),
        Some(Err(line)) => return Err(MarketError::bad(line)),
        Some(Ok(catalog)) => catalog,
    };
    let Some(entry) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&catalog).cloned())
        .filter(|e| e.representation == Representation::Asset)
    else {
        return Err(MarketError::bad(NOT_LISTABLE));
    };
    if crate::trade::asset_on_sale(ctx.w, spec.object) {
        return Err(MarketError::bad(ALREADY_FOR_SALE));
    }
    if spec.price.minor() <= 0 {
        return Err(MarketError::bad("A listing asks a positive price."));
    }
    check_private(ctx.w, &spec.seller, spec.private_to.as_ref())?;
    let Some(account) = principal_account(ctx, &spec.instrument, &spec.seller) else {
        return Err(MarketError::bad(NO_ACCOUNT));
    };
    if let Some(gate) = crate::module::venue_gate(ctx.w) {
        gate.admit(
            ctx.w,
            actor,
            &spec.market,
            None,
            crate::module::Presence::Here,
        )
        .map_err(|r| MarketError::bad(r.line))?;
    }
    check_caps(ctx.w, &spec.seller)?;
    let ttl = spec.ttl_secs.clamp(60, crate::api::MAX_LISTING_TTL_SECS);
    let now = ctx.w.now();
    let Some(id) = ctx
        .w
        .module_state_mut::<MarketStore>()
        .and_then(MarketStore::alloc_listing)
    else {
        return Err(MarketError::bad("The listing-id allocator is exhausted."));
    };
    // Doc 48 cut EX.4 (Q-48-8): the listing fee, once, the last fallible step.
    crate::fees::charge_listing_fee(ctx, &spec.seller, account, &spec.market, id, spec.price)?;
    put_listing(
        ctx,
        Listing {
            id,
            seller: spec.seller,
            vendor: NO_VENDOR,
            market: spec.market,
            catalog: entry.catalog.clone(),
            definition_version: entry.definition_version,
            source: Supply::Stock {
                vault: crate::record::NO_VAULT,
            },
            object: Some(spec.object),
            price: spec.price,
            quantity: 1,
            visibility: if spec.private_to.is_some() {
                MarketVisibility::Hidden
            } else {
                MarketVisibility::Public
            },
            expires: EconomicDeadline {
                clock: EconomicClock::RealUtc,
                at: now.saturating_add(ttl),
            },
            revision: 1,
            draw: Some(false),
            account: Some(account),
            private_to: spec.private_to,
            escrow: None,
        },
    );
    persist_meta(ctx);
    Ok(id)
}

// ---- a bid: escrowed at posting ------------------------------------------------

/// What [`post_bid`] posts (doc 48 cut EX.1a).
#[derive(Clone, Debug)]
pub struct PersonalBid {
    /// Who bids: the actor.
    pub bidder: PrincipalId,
    /// The venue.
    pub market: MarketId,
    /// The venue's currency, which the escrow is held in.
    pub instrument: InstrumentId,
    /// What it buys: a lot-represented entry whose code is a commodity id.
    pub catalog: CatalogRef,
    /// Where bought goods are received: an endpoint the actor may take from.
    pub into: mush_inventory::EndpointRef,
    /// The bid per unit.
    pub price: Amount,
    /// How many it wants, `1..=MAX_QUANTITY`.
    pub quantity: u64,
    /// A private bid's one seller.
    pub private_to: Option<PrincipalId>,
    /// How long it stands, in real seconds (clamped to `60..=MAX_LISTING_TTL_SECS`).
    pub ttl_secs: i64,
}

/// The hold purpose a personal bid's escrow carries.
#[must_use]
pub fn bid_purpose(listing: ListingId) -> mush_economy::HoldPurpose {
    mush_economy::HoldPurpose::Reservation {
        kind: smol_str::SmolStr::new_static(BID_ESCROW_KIND),
        id: listing.0,
    }
}

/// **Post a bid, escrowed** (doc 48 cut EX.1a; Q-48-50): a `Supply::Wanted` listing whose
/// seller is the bidder, backed by a reservation hold of `price × quantity` on the
/// bidder's own account, placed under the bidder's own debit authority and lapsing at the
/// listing's own deadline.
///
/// Refused before any write: a bidder other than the actor; an entry that is not
/// lot-represented or whose code is not a commodity id; a receiving vault the actor may not
/// take from; a non-positive price, an out-of-range quantity, a total that overflows; a
/// bulk good received into a governed hold ([`BID_INTO_HOLD`]); the venue gate; a second
/// live bid of this bidder for this good; a private bid naming nobody else; no account in
/// the venue's currency; the caps. The hold's post is the last fallible step; the listing
/// is written only once it is held.
///
/// # Errors
///
/// [`MarketError`]; render it with [`crate::refuse_market`].
pub fn post_bid(ctx: &mut Ctx<'_>, spec: PersonalBid) -> Result<ListingId, MarketError> {
    let actor = ctx.actor;
    if spec.bidder != PrincipalId::Object(actor) {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    }
    let Some(entry) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&spec.catalog).cloned())
    else {
        return Err(MarketError::NoStanding(crate::api::NO_SUCH_ENTRY));
    };
    if entry.representation != Representation::Lot {
        return Err(MarketError::bad(
            "A standing bid buys a lot-represented good.",
        ));
    }
    let Ok(commodity) = mush_core::CanonicalId::new(spec.catalog.code.as_str()) else {
        return Err(MarketError::bad("That catalog code is not a commodity id."));
    };
    if !mush_inventory::may_take(ctx.w, actor, spec.into) {
        return Err(MarketError::NoStanding(NO_SUCH_VAULT));
    }
    if spec.price.minor() <= 0 {
        return Err(MarketError::bad("A standing bid offers a positive price."));
    }
    if spec.quantity == 0 || spec.quantity > MAX_QUANTITY {
        return Err(MarketError::bad("That is not a quantity this market buys."));
    }
    let Some(total) = total_of(spec.price, spec.quantity) else {
        return Err(MarketError::bad("That is not a quantity this market buys."));
    };
    let gate = crate::module::venue_gate(ctx.w);
    if let Some(gate) = &gate {
        if gate.governed(ctx.w, spec.into) && !crate::module::sells_as_lots(ctx.w, &commodity) {
            return Err(MarketError::bad(BID_INTO_HOLD));
        }
        gate.admit(
            ctx.w,
            actor,
            &spec.market,
            Some(spec.into),
            crate::module::Presence::Here,
        )
        .map_err(|r| MarketError::bad(r.line))?;
    }
    let twice = ctx.w.module_state::<MarketStore>().and_then(|s| {
        s.personal_listings_of(&spec.bidder)
            .find(|l| matches!(&l.source, Supply::Wanted { commodity: c, .. } if c.0 == commodity))
            .map(|l| l.id)
    });
    if let Some(other) = twice {
        return Err(MarketError::bad(
            BID_TWICE.replace("{listing}", &other.0.to_string()),
        ));
    }
    check_private(ctx.w, &spec.bidder, spec.private_to.as_ref())?;
    let Some(account) = principal_account(ctx, &spec.instrument, &spec.bidder) else {
        return Err(MarketError::bad(NO_ACCOUNT));
    };
    check_caps(ctx.w, &spec.bidder)?;
    let ttl = spec.ttl_secs.clamp(60, crate::api::MAX_LISTING_TTL_SECS);
    let deadline = EconomicDeadline {
        clock: EconomicClock::RealUtc,
        at: ctx.w.now().saturating_add(ttl),
    };
    let Some(id) = ctx
        .w
        .module_state_mut::<MarketStore>()
        .and_then(MarketStore::alloc_listing)
    else {
        return Err(MarketError::bad("The listing-id allocator is exhausted."));
    };
    persist_meta(ctx);
    let fee = crate::fees::listing_fee_accounts(ctx.w, &spec.market, &spec.bidder).map(|venue| {
        (
            crate::fees::listing_fee(total, venue.listing_floor),
            venue.operator,
        )
    });
    let fee_lines = match fee {
        Some((fee, operator)) if fee.minor() > 0 => crate::fees::fee_lines(account, operator, fee),
        _ => Vec::new(),
    };
    // **The escrow, under the bidder's own authority** — the last fallible call.
    let plan = mush_economy::EconomyPlan {
        hold_ops: vec![mush_economy::HoldOp::Place {
            account,
            amount: total,
            beneficiary: None,
            purpose: bid_purpose(id),
            expires: Some(deadline),
        }],
        description: format!("market bid #{id} escrow"),
        ..mush_economy::EconomyPlan::new(
            mush_economy::IdempotencyKey::new(
                smol_str::SmolStr::new_static("market.bid"),
                smol_str::SmolStr::new(id.0.to_string()),
            ),
            spec.bidder.clone(),
            mush_economy::EconomicCause::Command {
                serial: ctx.w.next_correlation_id(),
            },
            // Doc 48 cut EX.4 (Q-48-8): the listing fee, once, in the escrow's own post.
            fee_lines,
        )
    };
    let rendered = crate::commands::money(ctx.w, Some(&spec.instrument), total);
    match mush_economy::post(ctx, &plan) {
        Ok(mush_economy::PostOutcome::Posted(_) | mush_economy::PostOutcome::AlreadyPosted(_)) => {}
        Ok(mush_economy::PostOutcome::NeedsApproval(_)) => {
            return Err(MarketError::bad(BID_NEEDS_APPROVAL));
        }
        Err(
            mush_economy::EconomyError::InsufficientAvailable(_)
            | mush_economy::EconomyError::CreditViolation(_),
        ) => {
            return Err(MarketError::bad(
                BID_CANNOT_COVER.replace("{total}", &rendered),
            ));
        }
        Err(error) => {
            tracing::info!(listing = id.0, %error, "market: a bid's escrow was refused");
            return Err(MarketError::bad(BID_NOT_HELD));
        }
    }
    let Some(hold) = bid_hold(ctx.w, id) else {
        tracing::error!(
            listing = id.0,
            "market: a bid's escrow posted and cannot be found"
        );
        return Err(MarketError::bad(BID_NOT_HELD));
    };
    put_listing(
        ctx,
        Listing {
            id,
            seller: spec.bidder,
            vendor: NO_VENDOR,
            market: spec.market,
            catalog: spec.catalog,
            definition_version: entry.definition_version,
            source: Supply::Wanted {
                vault: spec.into.vault(),
                commodity: LotCommodity(commodity),
                min_grade: 0,
            },
            object: None,
            price: spec.price,
            quantity: spec.quantity,
            visibility: if spec.private_to.is_some() {
                MarketVisibility::Hidden
            } else {
                MarketVisibility::Public
            },
            expires: deadline,
            revision: 1,
            draw: Some(false),
            account: Some(account),
            private_to: spec.private_to,
            escrow: Some(hold.id),
        },
    );
    Ok(id)
}

/// **A personal bid's live escrow**, found by its purpose (the industry budget hold's
/// rule: by purpose, never by an id alone), `None` when it is not `Held`.
#[must_use]
pub fn bid_hold(w: &World, listing: ListingId) -> Option<mush_economy::Hold> {
    let want = bid_purpose(listing);
    w.module_state::<EconomyStore>()?
        .snapshot()
        .holds()
        .find(|h| h.purpose == want && h.status.is_held())
        .cloned()
}

/// Whether a personal bid's escrow still covers `quantity` more at its price: the hold the
/// listing names, live, on its account, for at least `price × quantity`.
#[must_use]
pub fn bid_funds(w: &World, listing: &Listing, quantity: u64) -> bool {
    let (Some(escrow), Some(account)) = (listing.escrow, listing.account) else {
        return false;
    };
    let Some(need) = total_of(listing.price, quantity) else {
        return false;
    };
    w.module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().hold(escrow).cloned())
        .is_some_and(|h| h.status.is_held() && h.account == account && h.amount >= need)
}

// ---- withdrawal --------------------------------------------------------------------

/// **Withdraw one's own personal listing** (doc 48 cut EX.1a): an ask gives its lot back
/// (the release leg `withdraw_listing` stages), a bid releases what its escrow still
/// holds. Only the seller, or someone who acts for the selling organization; anyone else,
/// and a vendor listing, reads [`NO_SUCH_LISTING`].
///
/// # Errors
///
/// [`MarketError::NoStanding`] when there is no such listing of the actor's.
pub fn withdraw_own(ctx: &mut Ctx<'_>, id: ListingId) -> Result<Listing, MarketError> {
    let Some(listing) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.listing(id).cloned())
        .filter(|l| l.is_personal())
    else {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    };
    if !mush_economy::acts_for(ctx.w, ctx.actor, &listing.seller) {
        return Err(MarketError::NoStanding(NO_SUCH_LISTING));
    }
    crate::api::withdraw_listing(ctx, id)?;
    Ok(listing)
}

/// **Release a withdrawn personal bid's escrow** (doc 48 cut EX.1a), as the withdrawing
/// actor: the bidder is the hold's placer, so its release is theirs. The listing is
/// already deleted, so this post is the last fallible step; a refusal is logged and the
/// hold still lapses at its own deadline.
pub(crate) fn release_bid_escrow(ctx: &mut Ctx<'_>, listing: &Listing) {
    let Some(escrow) = listing.escrow else {
        return;
    };
    let live = ctx
        .w
        .module_state::<EconomyStore>()
        .and_then(|s| s.snapshot().hold(escrow).map(|h| h.status.is_held()))
        .unwrap_or(false);
    if !live {
        return;
    }
    let plan = mush_economy::EconomyPlan {
        hold_ops: vec![mush_economy::HoldOp::Release { hold: escrow }],
        description: format!("market bid #{} withdrawn", listing.id),
        ..mush_economy::EconomyPlan::new(
            mush_economy::IdempotencyKey::new(
                smol_str::SmolStr::new_static("market.bid.release"),
                smol_str::SmolStr::new(format!("{}/{}", listing.id.0, escrow.0)),
            ),
            PrincipalId::Object(ctx.actor),
            mush_economy::EconomicCause::Command {
                serial: ctx.w.next_correlation_id(),
            },
            Vec::new(),
        )
    };
    if let Err(error) = mush_economy::post(ctx, &plan) {
        tracing::info!(
            listing = listing.id.0,
            %error,
            "market: a withdrawn bid's escrow was not released; it lapses at its deadline"
        );
    }
}

// ---- standing acceptance -------------------------------------------------------

/// **A listing is its poster's standing acceptance** (doc 48 Q-48-50, cut EX.1a): the
/// market's implementation of `mush_economy::StandingAcceptance`, wired once at
/// composition beside the market's dispute guard.
///
/// It says yes for a contract only when all of these hold, and no otherwise:
///
/// - the contract is a market quote's, and that quote's listing **still stands**: present
///   (a withdrawal deletes it), personal, not lapsed, at the **same revision** the quote
///   saw, and its seller trading;
/// - the quote's terms are the listing's: its **price** the listing's price, its
///   **quantity** within the listing's, and for a private offer its counterparty the one
///   named;
/// - `party` is the listing's seller, and the contract's terms are the quote's:
///   - **an ask** (the party sells): the buyer, the quote's, is the contract's issuer and
///     its only payer, owing exactly `price × quantity`, the seller owing nothing, and no
///     backing hold named;
///   - **a bid** (the party buys): the party owes exactly `price × quantity` from the
///     bid's own account, and `against` is exactly the bid's escrow hold — or, asked of
///     the draft at the offer (where the seller only names the bid's account), none.
#[derive(Clone, Copy, Debug, Default)]
pub struct ListingAcceptance;

impl mush_economy::StandingAcceptance for ListingAcceptance {
    fn stands(
        &self,
        w: &World,
        contract: &EconomicContract,
        party: &PrincipalId,
        against: Option<HoldId>,
    ) -> bool {
        let Some(store) = w.module_state::<MarketStore>() else {
            return false;
        };
        let Some(quote) = store
            .quote_of_contract(contract.id)
            .and_then(|q| store.quote(q))
        else {
            return false;
        };
        let Some(listing) = store.listing(quote.listing) else {
            return false;
        };
        if !listing.is_personal()
            || listing.seller != *party
            || !personal_trading(w, listing)
            || crate::api::listing_lapsed(listing, w.now())
            || listing.revision != quote.observed.listing
            || quote.price != listing.price
            || !crate::trade::sells_quantity(listing, quote.quantity)
        {
            return false;
        }
        let Some(total) = total_of(quote.price, quote.quantity) else {
            return false;
        };
        let owed = |who: &PrincipalId| -> Vec<(AccountId, Amount)> {
            contract
                .consent
                .terms
                .consideration
                .iter()
                .filter(|c| c.from == *who)
                .map(|c| (c.account, c.amount))
                .collect()
        };
        if listing.source.is_wanted() {
            // A bid: the party is the buyer, paying from the bid's own escrow.
            let private_ok = listing
                .private_to
                .as_ref()
                .is_none_or(|to| *to == quote.seller);
            // At the offer the surface is still a draft and the seller only names the bid's
            // account, drawing nothing; at the acceptance it draws on exactly the bid's own
            // escrow.
            let backing_ok = match contract.state {
                mush_economy::ContractState::Draft => against.is_none(),
                _ => against.is_some() && against == listing.escrow,
            };
            // Doc 48e (exchange G26; 48d M-07): every outcome pays exactly what the market
            // builds for a sale into a person's bid: the seller the rest, then the venue's cut
            // and levy and the quote's frozen proceeds cuts.
            let fee_rows = crate::fees::sale_fee_rows(w, &listing.market, total);
            let outcomes = &contract.consent.terms.outcomes;
            let pays_as_built = !outcomes.is_empty()
                && outcomes.iter().all(|o| {
                    o.payouts.first().is_some_and(|first| {
                        crate::fees::bid_sale_payouts(first.to, total, &fee_rows, &quote.cuts)
                            .is_some_and(|expected| o.payouts == expected)
                    })
                });
            quote.buyer == *party
                && private_ok
                && backing_ok
                && pays_as_built
                && listing
                    .account
                    .is_some_and(|account| owed(party) == vec![(account, total)])
                && contract.consent.terms.consideration.len() == 1
        } else {
            // An ask: the party is the seller, owing nothing; the buyer pays it all.
            let private_ok = listing
                .private_to
                .as_ref()
                .is_none_or(|to| *to == quote.buyer);
            let paid = owed(&quote.buyer)
                .iter()
                .try_fold(0_i64, |sum, (_, a)| sum.checked_add(a.minor()));
            // Every outcome pays exactly what a person's sale pays (doc 48 cut EX.4): the
            // seller's own account the price less the venue's cut and levy and any proceeds
            // policy's share, and each of those exactly its own.
            let pays_seller = listing.account.is_some_and(|account| {
                crate::fees::personal_ask_payouts(
                    w,
                    listing,
                    &quote.buyer,
                    quote.quantity,
                    quote.price,
                    account,
                )
                .is_ok_and(|expected| {
                    contract
                        .consent
                        .terms
                        .outcomes
                        .iter()
                        .all(|o| o.payouts == expected)
                })
            });
            quote.seller == *party
                && private_ok
                && against.is_none()
                && contract.issuer == quote.buyer
                && paid == Some(total.minor())
                // Every consideration is the buyer's: the seller owes nothing.
                && contract.consent.terms.consideration.len() == owed(&quote.buyer).len()
                && pays_seller
        }
    }
}

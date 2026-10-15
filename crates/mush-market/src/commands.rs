//! The `+market` and `@market` surfaces (E6c, §5.3's grammar).
//!
//! Two primary verbs, both entered in the pinned rule-1 allowlist in the same commit
//! (08i §4, B11). Everything they do goes through [`crate::api`] and
//! [`crate::trade`], and every refusal goes through the existence-hiding renderer:
//! `No such listing.` / `No such quote.` / `No such vendor.`, identical for
//! nonexistent and unviewable, and a hidden vendor's **listings are absent** rather
//! than refused.
//!
//! `+market/depth` is the one read with no viewer in it at all: it renders
//! [`MarketStore::projection`](crate::MarketStore::projection), which takes no viewer
//! argument, so a staffer and a stranger get byte-identical output by construction.

use std::sync::Arc;

use mush_core::module::{ModError, Registrar};
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx};
use mush_economy::{Amount, CatalogRef};
use smol_str::SmolStr;

use crate::api::{
    self, MarketError, MarketFilter, NO_SUCH_ENTRY, NO_SUCH_LISTING, NO_SUCH_QUOTE, NO_SUCH_VENDOR,
    may_view_listing, refuse_market,
};
use crate::ids::{ListingId, MarketId, MarketSourceId, QuoteId, VendorId, parse_u64};
use crate::record::{MarketVisibility, Supply};
use crate::store::{MARKET_PAGE, MarketStore};
use crate::trade::{QuoteRequest, QuoteWindow};

/// Register the two market verbs.
///
/// # Errors
///
/// [`ModError`] if a name is already taken.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    // ---- E7r2: P1 — both verbs publish `Authoritative`, per group ----
    //
    // K8.5's rule is `classify_commands` **per group, before the first `reg.command`
    // of that group**, and this crate is the one the rule was written from: it failed
    // 140 tests at the K merge for want of it. Both verbs move
    // `LegacyPostImage -> Authoritative` by the **ceiling** rule a `CommandSpec` takes
    // over its switches (`@station`, `@inv`, `@wo`, `@prop`, `@domain`): `@market/vendor
    // stock` and `/withdraw` and `+market/accept` all stage
    // `Ctx::submit_authoritative` of the market-owned lot action. The other switches
    // read, or write the module's own `market:*` documents, and they ride the same
    // lane because a primary verb has one class.
    //
    // `LegacyPostImage` is **restored last**, so the class this module ends in is the
    // one its own documents ride — cut 9's campaign lane, exactly.
    reg.classify_commands(mush_core::CommandCompatibility::Authoritative);
    reg.command(
        CommandSpec {
            name: "+market".into(),
            shape: ArgShape::EqPair,
            switches: [
                "list", "show", "depth", "quote", "accept", "sell", "orders", "vendors",
            ]
            .iter()
            .copied()
            .map(SmolStr::new)
            .collect(),
            help: "+market/list [legal=<class>|tag=<tag>|market=<venue>] [after=<id>]\n\
                   \x20                                 what is for sale, newest first, one \
                   bounded page at a time. A filter you may not use simply shows nothing.\n\
                   +market/show <listing>             one listing.\n\
                   +market/depth <ruleset>:<code> [market=<venue>]\n\
                   \x20                                 the PUBLIC picture of a good: last trade, \
                   a price band, bucketed depth and the window's volume. It names no account, no \
                   listing, no counterparty and no seller's quantity, and it is the same for \
                   everyone who asks.\n\
                   +market/quote <listing>[=<qty>] [remote]\n\
                   \x20                                 a price with a declared expiry. It \
                   reserves nothing: the goods are still anyone's until a deal is in force.\n\
                   +market/accept <quote>[=<endpoint>]\n\
                   \x20                                 take it up. You are the issuer: your \
                   escrow is placed from your own account, and the vendor accepts on its own \
                   standing. A listing sold out of an inventory endpoint is delivered INTO one, \
                   so name a vault you may draw from.\n\
                   +market/sell <obj>=<vendor>        offer something you are carrying. The \
                   vendor escrows its payment first; you then accept the contract and \
                   deposit the goods into its vault (@vault/deposit). It buys CUSTODY, so \
                   your title record survives the sale.\n\
                   +market/orders                     your own open quotes and deals.\n\
                   +market/vendors                    who is trading."
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(market_cmd),
    )?;
    reg.command(
        CommandSpec {
            name: "@market".into(),
            shape: ArgShape::EqPair,
            switches: ["catalog", "vendor", "source"]
                .iter()
                .copied()
                .map(SmolStr::new)
                .collect(),
            help: "@market/catalog load <pack>        load or reload a content pack. A bad entry \
                   fails the PACK and names it; the server keeps running.\n\
                   @market/catalog list [<pack>]      what is in the catalogue.\n\
                   @market/catalog show <ruleset>:<code>\n\
                   @market/vendor create <org>=<vault> [name=<text>] [market=<venue>] [buy=<permille>]\n\
                   @market/vendor fund <vendor>=<amount> [faucet=<id>] [period=<secs>]\n\
                   \x20                                 the ONLY route by which value enters a \
                   vendor other than a sale: a registered faucet's drip under the [economy] mint \
                   cap.\n\
                   @market/vendor stock <vendor>=<ruleset>:<code> price=<n> [qty=<n>] \
                   [object=<obj>] [hidden] [days=<n>] [lead=<secs>]\n\
                   \x20                                 …or, for a lot-represented good, \
                   [endpoint=<vault>] [commodity=<id>] [grade=<n>] — which RESERVES that much of \
                   the endpoint's stock until the listing sells or is withdrawn. An \
                   organization's own officer may stock and withdraw its lot listings; \
                   everything else here is staff.\n\
                   @market/vendor withdraw <listing>  take a listing down and release anything it \
                   held.\n\
                   @market/vendor retire <vendor>     stop it trading. Live contracts still settle.\n\
                   @market/source create <org>=<vault> [market=<venue>] [spread=<permille>] \
                   [volume=<n>] [loss=<n>] [window=<secs>]\n\
                   @market/source budget <source>=[volume=<n>] [loss=<n>] [spread=<permille>]\n\
                   @market/source halt <source> [on|off]"
                .into(),
            ..CommandSpec::default()
        },
        Arc::new(market_admin),
    )?;
    reg.classify_commands(mush_core::CommandCompatibility::LegacyPostImage);
    Ok(())
}

fn info(ctx: &mut Ctx<'_>, msg: impl Into<String>) {
    let actor = ctx.actor;
    ctx.out.sys_to_actor(actor, "market", msg.into());
}

fn lines(ctx: &mut Ctx<'_>, v: Vec<String>) {
    for l in v {
        info(ctx, l);
    }
}

fn require_staff(ctx: &Ctx<'_>) -> Result<(), CmdError> {
    if api::is_staff(ctx.w, ctx.actor) {
        Ok(())
    } else {
        Err(CmdError::msg("Permission denied."))
    }
}

/// Parse `<ruleset>:<code>` into a [`CatalogRef`] — the same grammar
/// `@asset/title … catalog=` takes (E6b).
fn parse_catalog(s: &str) -> Option<CatalogRef> {
    let (r, c) = s.trim().split_once(':')?;
    let (r, c) = (r.trim(), c.trim());
    if r.is_empty() || c.is_empty() {
        return None;
    }
    Some(CatalogRef {
        ruleset: SmolStr::new(r),
        code: SmolStr::new(c),
    })
}

/// The instrument this market trades in — derived from the vendor whose deal is
/// being priced, never configured here: `mush-market` is game-neutral and names no
/// currency of its own.
fn vendor_instrument(ctx: &Ctx<'_>, vendor: VendorId) -> Option<mush_economy::InstrumentId> {
    let account = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor(vendor).map(|v| v.account))?;
    api::account_instrument(ctx, account)
}

// ---- +market ---------------------------------------------------------------

fn market_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if ctx.switches.has("depth") {
        depth(ctx)
    } else if ctx.switches.has("show") {
        show(ctx)
    } else if ctx.switches.has("quote") {
        quote(ctx)
    } else if ctx.switches.has("accept") {
        accept(ctx)
    } else if ctx.switches.has("sell") {
        sell(ctx)
    } else if ctx.switches.has("orders") {
        orders(ctx)
    } else if ctx.switches.has("vendors") {
        vendors(ctx)
    } else {
        list(ctx)
    }
}

fn list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // The whole argument, not `lhs()`: this verb's grammar is `key=value` words and
    // `ArgShape::EqPair` splits on the first `=`, so `legal=restricted` would arrive
    // as lhs "legal" and rhs "restricted" and match nothing.
    let arg = ctx.args.raw().trim().to_string();
    let mut filter: Option<MarketFilter> = None;
    let mut after: Option<ListingId> = None;
    for word in arg.split_whitespace() {
        if let Some(rest) = word.strip_prefix("after=") {
            after = parse_u64(rest).map(ListingId);
        } else {
            // An unknown filter word parses to `MarketFilter::Nothing`, which matches
            // nothing — an EMPTY LIST, never a refusal (08a decision 13).
            filter = Some(MarketFilter::parse(word));
        }
    }
    let page = api::visible(ctx.w, ctx.actor, filter.as_ref(), MARKET_PAGE, after);
    let mut out: Vec<String> = Vec::new();
    for id in &page.ids {
        if let Some(l) = ctx
            .w
            .module_state::<MarketStore>()
            .and_then(|s| s.listing(*id).cloned())
        {
            out.push(listing_line(ctx, &l));
        }
    }
    // **The cursor is unconditional on the walk, never on the match** (independent
    // verify fold, D3). `api::visible` is correctly bounded on records *visited* and
    // returns the last record visited, so the API pages in step for every viewer — but
    // the render only printed the cursor in the `else` after the empty case, so a viewer
    // whose 256 newest live listings were all unviewable got "Nothing is for sale." with
    // the cursor **discarded** and every older public listing unreachable from the
    // player surface. Five vendors at `MAX_LISTINGS_PER_VENDOR` build that wall, so
    // hidden state denied public state.
    if out.is_empty() {
        out.push("Nothing is for sale.".to_string());
    }
    if let Some(next) = page.next {
        out.push(format!("(more: +market/list after={next})"));
    }
    lines(ctx, out);
    Ok(())
}

// ---- doc 18 cut IC0: the one money render ------------------------------------

/// **The one money render in the market** (doc 18 cut IC0). Every price, total, band,
/// volume and baseline a market verb or the economist prints goes through here, so a
/// 700-centicredit price reads in the instrument's own scale and symbol and never as
/// the bare minor value the verbs used to print. It is [`mush_economy::render_amount`]
/// — the economy's one player-facing render — on the instrument the record settles
/// in; the bare minor value is kept only for an amount nothing knows the instrument
/// of, because a render never refuses. The parse side (`price=<n>`) still takes minor
/// units, exactly as every economy verb does.
#[must_use]
pub(crate) fn money(
    w: &mush_core::World,
    instrument: Option<&mush_economy::InstrumentId>,
    amount: Amount,
) -> String {
    match (w.module_state::<mush_economy::EconomyStore>(), instrument) {
        (Some(economy), Some(instrument)) => {
            mush_economy::render_amount(economy.snapshot(), instrument, amount)
        }
        _ => amount.to_decimal_string(0),
    }
}

/// The instrument a vendor's deals settle in: its own cash account's.
#[must_use]
pub(crate) fn vendor_instrument_of(
    w: &mush_core::World,
    vendor: VendorId,
) -> Option<mush_economy::InstrumentId> {
    let account = w.module_state::<MarketStore>()?.vendor(vendor)?.account;
    w.module_state::<mush_economy::EconomyStore>()?
        .snapshot()
        .account(account)
        .map(|a| a.instrument.clone())
}

/// The instrument a listing settles in.
#[must_use]
pub(crate) fn listing_instrument(
    w: &mush_core::World,
    l: &crate::record::Listing,
) -> Option<mush_economy::InstrumentId> {
    vendor_instrument_of(w, l.vendor)
}

/// The instrument a quote settles in: its listing's while the listing stands, and the
/// seller's vendor record's once it is gone — a lot listing is deleted by its one
/// sale, and a settled quote is the tape row that must describe itself afterwards.
#[must_use]
pub(crate) fn quote_instrument(
    w: &mush_core::World,
    q: &crate::record::Quote,
) -> Option<mush_economy::InstrumentId> {
    let store = w.module_state::<MarketStore>()?;
    let vendor = store
        .listing(q.listing)
        .map(|l| l.vendor)
        .or_else(|| match &q.seller {
            mush_org::PrincipalId::Organization(org) => store.vendor_of_org(*org).map(|v| v.id),
            mush_org::PrincipalId::Object(_) | mush_org::PrincipalId::System(_) => None,
        })?;
    vendor_instrument_of(w, vendor)
}

/// The instrument a venue trades in: that of its first vendor by id. One instrument
/// per venue is the market's working assumption (a listing settles in its vendor's,
/// and the projection sums prices across vendors as if they agreed); a venue with no
/// vendor renders bare minor.
#[must_use]
pub(crate) fn market_instrument(
    w: &mush_core::World,
    market: &MarketId,
) -> Option<mush_economy::InstrumentId> {
    let vendor = w
        .module_state::<MarketStore>()?
        .vendors()
        .find(|v| &v.market == market)?
        .id;
    vendor_instrument_of(w, vendor)
}

/// The instrument a catalogue **baseline** is read in: the first vendor's by id, there
/// being no venue in a catalogue entry. A reference price in a world with no vendor
/// renders bare minor.
#[must_use]
pub(crate) fn catalog_instrument(w: &mush_core::World) -> Option<mush_economy::InstrumentId> {
    let vendor = w.module_state::<MarketStore>()?.vendors().next()?.id;
    vendor_instrument_of(w, vendor)
}

pub(crate) fn listing_line(ctx: &Ctx<'_>, l: &crate::record::Listing) -> String {
    let name = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&l.catalog).map(|e| e.name.to_string()))
        .unwrap_or_else(|| l.catalog.code.to_string());
    let shop = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor(l.vendor).map(|v| v.name.to_string()))
        .unwrap_or_default();
    // **The row advertises the price the deal executes at** (independent verify fold,
    // D4): a maker's spread moved `Quote.price`, and this line read `Listing.price`
    // unmoved, so `+market/list` advertised a price nobody pays.
    let price = crate::trade::price_for(l, ctx.w.module_state::<MarketStore>());
    let instrument = listing_instrument(ctx.w, l);
    format!(
        "#{} {name} — {} × {} [{}]{}{}",
        l.id,
        l.quantity,
        money(ctx.w, instrument.as_ref(), price),
        l.market,
        if shop.is_empty() {
            String::new()
        } else {
            format!(" at {shop}")
        },
        match l.visibility {
            MarketVisibility::Public => "",
            MarketVisibility::Hidden => " (hidden)",
        }
    )
}

fn show(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(id) = parse_u64(ctx.args.lhs()).map(ListingId) else {
        return Err(refuse(ctx, NO_SUCH_LISTING));
    };
    let Some(l) = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.listing(id).cloned())
    else {
        return Err(refuse(ctx, NO_SUCH_LISTING));
    };
    // Identical text for nonexistent and unviewable — the whole of 08a decision 13.
    if !may_view_listing(ctx.w, ctx.actor, &l) {
        return Err(refuse(ctx, NO_SUCH_LISTING));
    }
    let line = listing_line(ctx, &l);
    let extra = match &l.source {
        Supply::Stock { .. } => "in stock".to_string(),
        Supply::Market { .. } => "quoted by a market maker".to_string(),
        Supply::Procurement { lead } => format!("procured, delivered by {}", lead.at),
        // E7r2 P1: the handles are OPAQUE. A viewer learns that the stock is real and
        // already held — which is the honest thing a reserving listing says — and
        // learns no endpoint, no commodity id and no other principal's inventory.
        Supply::Lot { .. } => "held stock, reserved for this listing".to_string(),
    };
    lines(ctx, vec![line, format!("  supply: {extra}")]);
    Ok(())
}

/// `+market/depth <ruleset>:<code> [market=<venue>]` — **the gate's own render**.
///
/// It takes the actor's name for nothing: the projection function has no viewer
/// parameter, so this renders the same bytes for a staffer, a member and a stranger,
/// and there is no branch anywhere in it that could differ.
fn depth(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // The whole argument (see `list`): `market=<venue>` carries its own `=`.
    let arg = ctx.args.raw().trim().to_string();
    let mut words = arg.split_whitespace();
    let Some(catalog) = words.next().and_then(parse_catalog) else {
        return Err(refuse(ctx, NO_SUCH_ENTRY));
    };
    let mut market: Option<MarketId> = None;
    for w in words {
        if let Some(rest) = w.strip_prefix("market=") {
            market = MarketId::new(rest);
        }
    }
    let hidden = api::hidden_seller_orgs(ctx);
    // With no venue named, the venue of the first **public** listing or trade of that
    // good — and, when there is none, a fixed fallback rather than a different
    // message. Both halves matter. Reading every listing here would have made the
    // default an existence oracle for a hidden one (it would pick that listing's
    // venue), and answering "nothing trades in that" for the empty case would have
    // made the *shape* of the reply one, because a good with only hidden listings
    // would have rendered an empty projection where a good with none rendered a
    // sentence. Both now render the empty projection, byte for byte.
    let market = market
        .or_else(|| default_market(ctx, &catalog, &hidden))
        .or_else(|| MarketId::new("default"));
    let Some(market) = market else {
        return Err(refuse(ctx, NO_SUCH_ENTRY));
    };
    let now = ctx.w.now();
    // **No deep copy of the whole economy on a player verb** (independent verify fold,
    // D5). This held the only whole-snapshot clone in the tree, and `EconomySnapshot` is
    // some twenty owned maps — instruments, accounts and their indexes, idempotency,
    // assets, vaults, approvals, holds, cadences, obligations, contracts — with no `Arc`
    // and no copy-on-write, so `+market/depth` copied the whole world's economy on every
    // call by any player, on the single-writer world task, with no rate limit: 200 calls
    // went 2.8 ms → 31.3 ms after 300 extra *accounts*, with the market state unchanged.
    // The clone bought nothing either: both module states are shared reborrows of the
    // same `&World`, so they can simply be held at once.
    let economy = ctx.w.module_state::<mush_economy::EconomyStore>();
    let store = ctx.w.module_state::<MarketStore>();
    let (Some(economy), Some(store)) = (economy, store) else {
        return Err(refuse(ctx, NO_SUCH_ENTRY));
    };
    let p = store.projection(&catalog, &market, economy.snapshot(), &hidden, now);
    let instrument = market_instrument(ctx.w, &market);
    let render = |amount: Amount| money(ctx.w, instrument.as_ref(), amount);
    let mut out = vec![
        format!("{}:{} in {market}", catalog.ruleset, catalog.code),
        format!(
            "  last trade: {}   band: {}..{}   volume ({} d): {}",
            p.last_trade.map_or("—".to_string(), render),
            render(p.band.0),
            render(p.band.1),
            p.window_secs / 86_400,
            render(p.volume)
        ),
    ];
    let depth: Vec<String> = p
        .depth
        .iter()
        .filter(|b| b.quantity > 0)
        .map(|b| format!("{}×{}", render(b.price), b.quantity))
        .collect();
    out.push(format!(
        "  depth: {}",
        if depth.is_empty() {
            "—".to_string()
        } else {
            depth.join("  ")
        }
    ));
    lines(ctx, out);
    Ok(())
}

/// The venue a good trades in, when the caller named none.
///
/// Reads **exactly** the set the projection itself folds, through the projection's own
/// two predicates — never the viewer, and never a record the projection would have
/// ignored. A hidden listing must not be able to choose the venue any more than it can
/// move a number in it.
///
/// Two independent fixes (independent verify fold, D7 and D8):
///
/// - it applied two of the projection's four listing tests, so a **lapsed** public
///   listing still chose the header venue while contributing nothing to the numbers,
///   and a good whose only public listing had lapsed rendered a different first line
///   from a good with no listing at all. It calls `MarketStore::publishes` now, so the
///   two cannot drift again;
/// - it walked `store.listings` and then `store.quotes` — the **whole maps**, up to
///   1 024 + 2 048 records per call — rather than the pair index the projection uses.
///   Both walks are `range` scans scoped to the one catalogue now.
fn default_market(
    ctx: &Ctx<'_>,
    catalog: &CatalogRef,
    hidden: &[mush_org::OrgId],
) -> Option<MarketId> {
    let store = ctx.w.module_state::<MarketStore>()?;
    let now = ctx.w.now();
    let venues: Vec<MarketId> = store.markets_of_catalog(catalog).cloned().collect();
    for market in venues {
        if store
            .listings_for(catalog, &market)
            .any(|l| store.publishes(l, hidden, now))
        {
            return Some(market);
        }
    }
    let traded: Vec<MarketId> = store.markets_of_trades(catalog).cloned().collect();
    traded.into_iter().find(|market| {
        store
            .trades_for(catalog, market)
            .any(|q| store.publishes_trade(q, hidden))
    })
}

fn quote(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().trim().to_string();
    let rhs = ctx.args.rhs().unwrap_or_default().trim().to_string();
    let mut words = lhs.split_whitespace();
    let Some(listing) = words.next().and_then(parse_u64).map(ListingId) else {
        return Err(refuse(ctx, NO_SUCH_LISTING));
    };
    let remote = lhs.contains("remote") || rhs.contains("remote");
    let quantity = rhs
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(1);
    let req = QuoteRequest {
        listing,
        quantity,
        window: if remote {
            QuoteWindow::Remote
        } else {
            QuoteWindow::Live
        },
        ttl_live: crate::trade::QUOTE_TTL_LIVE,
        ttl_remote: crate::trade::QUOTE_TTL_REMOTE,
    };
    match crate::trade::quote(ctx, &req) {
        Ok(id) => {
            let q = ctx
                .w
                .module_state::<MarketStore>()
                .and_then(|s| s.quote(id).cloned());
            let msg = q.map_or_else(
                || format!("Quote #{id}."),
                |q| {
                    let instrument = quote_instrument(ctx.w, &q);
                    format!(
                        "Quote #{id}: {} × {} = {} — good until {} (real time). It reserves \
                         nothing; take it up with +market/accept #{id}.",
                        q.quantity,
                        money(ctx.w, instrument.as_ref(), q.price),
                        money(ctx.w, instrument.as_ref(), q.total()),
                        q.expires.at
                    )
                },
            );
            info(ctx, msg);
            Ok(())
        }
        Err(e) => Err(refuse_market(ctx, &e)),
    }
}

fn accept(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Some(id) = parse_u64(ctx.args.lhs()).map(QuoteId) else {
        return Err(refuse(ctx, NO_SUCH_QUOTE));
    };
    // E7r2 P1: `+market/accept <quote>=<vault>` names where a lot-represented purchase
    // is delivered. Every E6c representation ignores it, and a lot purchase that names
    // none is refused **before any money moves**.
    let destination = ctx
        .args
        .rhs()
        .and_then(|rhs| rhs.split_whitespace().next().and_then(parse_u64))
        .map(|id| mush_inventory::EndpointRef(mush_economy::VaultId(id)));
    match crate::trade::accept_into(ctx, id, destination) {
        Ok(contract) => {
            info(
                ctx,
                format!(
                    "Agreed. Contract #{contract} carries it; your payment is escrowed and \
                     settles when the terms are met."
                ),
            );
            Ok(())
        }
        Err(e) => Err(refuse_market(ctx, &e)),
    }
}

fn sell(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let Ok(object) = mush_core::match_object(ctx.w, ctx.actor, ctx.args.lhs().trim()) else {
        return Err(refuse(ctx, "No such asset."));
    };
    let rhs = ctx.args.rhs().unwrap_or_default().trim().to_string();
    let Some(vendor) = resolve_vendor(ctx, &rhs) else {
        return Err(refuse(ctx, NO_SUCH_VENDOR));
    };
    let Some(instrument) = vendor_instrument(ctx, vendor) else {
        return Err(refuse(ctx, NO_SUCH_VENDOR));
    };
    match crate::trade::sell(ctx, &instrument, object, vendor) {
        Ok(contract) => {
            info(
                ctx,
                format!(
                    "Offered. Contract #{contract} already holds the vendor's payment — the \
                     money is committed before the goods move. Accept it (@contract/accept \
                     #{contract}), then hand the goods over with @vault/deposit. The vendor \
                     buys CUSTODY: your title record, and anything recorded against it, \
                     survives the sale."
                ),
            );
            Ok(())
        }
        Err(e) => Err(refuse_market(ctx, &e)),
    }
}

fn orders(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let me = mush_org::PrincipalId::Object(ctx.actor);
    let mut out: Vec<String> = ctx
        .w
        .module_state::<MarketStore>()
        .map(|s| {
            s.quotes_of_buyer(&me)
                .take(crate::store::MARKET_PAGE_MAX)
                .map(|q| {
                    let instrument = quote_instrument(ctx.w, q);
                    format!(
                        "#{} {} × {} = {}{}",
                        q.id,
                        q.quantity,
                        money(ctx.w, instrument.as_ref(), q.price),
                        money(ctx.w, instrument.as_ref(), q.total()),
                        match (q.contract, q.settled) {
                            (_, Some(s)) => format!(" — settled ({})", s.transaction),
                            (Some(c), None) => format!(" — contract #{c}"),
                            (None, None) => format!(" — open until {}", q.expires.at),
                        }
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    if out.is_empty() {
        out.push("You have no orders.".to_string());
    }
    lines(ctx, out);
    Ok(())
}

fn vendors(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let viewer = ctx.actor;
    // Filter, then page (E6b verify fold D3): a hidden vendor is **absent**, so a
    // member's page and a non-member's page truncate the same way.
    let mut out: Vec<String> = ctx
        .w
        .module_state::<MarketStore>()
        .map(|s| {
            s.vendors()
                .filter(|v| v.is_trading() && api::may_list_vendor(ctx.w, viewer, v))
                .take(crate::record::MAX_VENDORS)
                .map(|v| format!("#{} {} — {}", v.id, v.name, v.market))
                .collect()
        })
        .unwrap_or_default();
    if out.is_empty() {
        out.push("Nobody is trading.".to_string());
    }
    lines(ctx, out);
    Ok(())
}

fn resolve_vendor(ctx: &Ctx<'_>, s: &str) -> Option<VendorId> {
    let s = s.trim();
    if let Some(n) = parse_u64(s) {
        let id = VendorId(n);
        return ctx
            .w
            .module_state::<MarketStore>()
            .and_then(|st| st.vendor(id))
            .filter(|v| api::may_list_vendor(ctx.w, ctx.actor, v))
            .map(|v| v.id);
    }
    let lower = s.to_ascii_lowercase();
    ctx.w
        .module_state::<MarketStore>()?
        .vendors()
        .find(|v| v.name.to_ascii_lowercase() == lower && api::may_list_vendor(ctx.w, ctx.actor, v))
        .map(|v| v.id)
}

/// One existence-hiding refusal, counted by the economy's C16 telemetry.
fn refuse(ctx: &mut Ctx<'_>, why: &'static str) -> CmdError {
    refuse_market(ctx, &MarketError::NoStanding(why))
}

// ---- @market ---------------------------------------------------------------

fn market_admin(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    // ---- E7r2: P1 — the authority is now PER SUB-VERB, not per command ----
    //
    // `@market` was staff-only for all three switches, and doc 10 §11's proving loop
    // asks that **no arrow needs a staff act** — a run that produced a lot has to be
    // sellable by the principal that produced it. Registering vendors, funding them
    // from a faucet, loading catalogue packs, registering market makers and retiring a
    // shop stay staff acts; `@market/vendor stock` and `/withdraw` become acts of the
    // vendor's **own organization** (`may_stock_for`), and only for a lot-represented
    // good — the arm that mints title as a side effect of stocking is still staff-only
    // and still checked inside `api::list_goods`.
    if ctx.switches.has("vendor") {
        return admin_vendor(ctx);
    }
    require_staff(ctx)?;
    if ctx.switches.has("catalog") {
        admin_catalog(ctx)
    } else if ctx.switches.has("source") {
        admin_source(ctx)
    } else {
        Err(CmdError::msg("Usage: @market/catalog|vendor|source …"))
    }
}

/// **Whether this actor may stock or withdraw for that vendor** (E7r2 P1).
///
/// Staff, or a principal `mush_economy::acts_for` answers true for over the vendor's
/// own organization — E3's own authority, asked through the economy's exported seam and
/// rebuilt nowhere. A vendor that does not exist answers `false`, so the refusal a
/// stranger gets is the one an unknown vendor gets.
fn may_stock_for(ctx: &Ctx<'_>, vendor: VendorId) -> bool {
    if api::is_staff(ctx.w, ctx.actor) {
        return true;
    }
    ctx.w
        .module_state::<MarketStore>()
        .and_then(|s| s.vendor(vendor).map(|v| v.org))
        .is_some_and(|org| {
            mush_economy::acts_for(ctx.w, ctx.actor, &mush_org::PrincipalId::Organization(org))
        })
}

fn admin_catalog(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().trim().to_string();
    let mut words = lhs.split_whitespace();
    let verb = words.next().unwrap_or("list").to_ascii_lowercase();
    let rest: Vec<&str> = words.collect();
    match verb.as_str() {
        "load" => {
            let Some(pack) = rest.first().copied() else {
                return Err(CmdError::msg("Usage: @market/catalog load <pack>"));
            };
            // **The pack name is validated before it becomes a filename** (independent
            // verify fold, D9). It went from `words` straight to
            // `SrCatalogSource::pack`, which does `direct.join(format!("{pack}.json"))`,
            // and `check_entry`'s byte rule runs only on the **entries** — after the
            // file has been opened, read and parsed. So `@market/catalog load
            // ../e6c-outside-probe` opened a file outside the configured root from the
            // world task, and the refusal echoed the resolved absolute path and told
            // "exists but is not a loadable pack" apart from "no such pack". It is the
            // rule `check_entry` already applies to `e.pack`, applied one step earlier,
            // and it renders the **same** refusal a missing pack gets so it is not an
            // existence oracle either.
            if !is_pack_name(pack) {
                return Err(CmdError::msg(format!(
                    "No content source on this server defines pack '{pack}'."
                )));
            }
            match crate::module::pack_entries(ctx, pack) {
                None => Err(CmdError::msg(format!(
                    "No content source on this server defines pack '{pack}'."
                ))),
                // A bad entry fails the PACK and names the file; the server keeps
                // running, which is the whole of B3's rule.
                Some(Err(why)) => Err(CmdError::msg(format!(
                    "Pack '{pack}' was not loaded: {why}"
                ))),
                Some(Ok(entries)) => match crate::api::load_pack(ctx, pack, entries) {
                    Ok(r) => {
                        info(
                            ctx,
                            format!(
                                "Pack '{}' loaded: {} entries ({} new, {} versioned, {} dropped).",
                                r.pack, r.entries, r.added, r.updated, r.dropped
                            ),
                        );
                        Ok(())
                    }
                    Err(e) => Err(refuse_market(ctx, &e)),
                },
            }
        }
        "list" => {
            let want = rest.first().copied();
            let mut out: Vec<String> = ctx
                .w
                .module_state::<MarketStore>()
                .map(|s| {
                    s.entries()
                        .filter(|e| want.is_none_or(|p| e.pack == p))
                        .take(crate::store::MARKET_PAGE_MAX)
                        .map(|e| {
                            format!(
                                "{}:{} v{} — {} [{} · {}]",
                                e.catalog.ruleset,
                                e.catalog.code,
                                e.definition_version,
                                e.name,
                                e.representation.label(),
                                e.legality.label()
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            if out.is_empty() {
                out.push("The catalogue is empty.".to_string());
            }
            lines(ctx, out);
            Ok(())
        }
        "show" => {
            let Some(catalog) = rest.first().copied().and_then(parse_catalog) else {
                return Err(CmdError::msg(
                    "Usage: @market/catalog show <ruleset>:<code>",
                ));
            };
            let Some(e) = ctx
                .w
                .module_state::<MarketStore>()
                .and_then(|s| s.entry_of(&catalog).cloned())
            else {
                return Err(refuse(ctx, NO_SUCH_ENTRY));
            };
            let out = vec![
                format!("{}:{} — {}", e.catalog.ruleset, e.catalog.code, e.name),
                format!(
                    "  pack {} v{} · {} · {} · {}",
                    e.pack,
                    e.definition_version,
                    e.representation.label(),
                    e.legality.label(),
                    if e.stackable { "stackable" } else { "single" }
                ),
                format!(
                    "  baseline (a REFERENCE price, never charged): {}",
                    e.baseline.map_or("—".to_string(), |b| money(
                        ctx.w,
                        catalog_instrument(ctx.w).as_ref(),
                        b
                    ))
                ),
                format!(
                    "  availability: {}",
                    match &e.availability {
                        crate::record::AvailabilityRule::None => "stocked".to_string(),
                        crate::record::AvailabilityRule::Attested => "attested".to_string(),
                        crate::record::AvailabilityRule::Test {
                            skill, threshold, ..
                        } => format!("test {skill} vs {threshold}"),
                    }
                ),
            ];
            lines(ctx, out);
            Ok(())
        }
        _ => Err(CmdError::msg("Usage: @market/catalog load|list|show …")),
    }
}

fn admin_vendor(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().trim().to_string();
    let rhs = ctx.args.rhs().unwrap_or_default().trim().to_string();
    let mut words = lhs.split_whitespace();
    let verb = words.next().unwrap_or("").to_ascii_lowercase();
    let target: Vec<&str> = words.collect();
    let opts = parse_opts(&rhs);
    match verb.as_str() {
        "create" => {
            let Some(org) = target
                .first()
                .and_then(|s| parse_u64(s))
                .map(mush_org::OrgId)
            else {
                return Err(CmdError::msg(
                    "Usage: @market/vendor create <org>=<vault> [name=] [market=] [buy=]",
                ));
            };
            require_staff(ctx)?;
            let Some(vault) = rhs
                .split_whitespace()
                .next()
                .and_then(parse_u64)
                .map(mush_economy::VaultId)
            else {
                return Err(CmdError::msg("Name the vendor's storefront vault."));
            };
            let market = opts
                .get("market")
                .and_then(|m| MarketId::new(m))
                .or_else(|| MarketId::new("default"));
            let Some(market) = market else {
                return Err(CmdError::msg("That is not a venue name."));
            };
            let instrument = ctx
                .w
                .module_state::<mush_economy::EconomyStore>()
                .and_then(|s| {
                    s.snapshot()
                        .accounts_of(&mush_org::PrincipalId::Organization(org))
                        .next()
                        .map(|a| a.instrument.clone())
                });
            let Some(instrument) = instrument else {
                return Err(refuse(ctx, NO_SUCH_VENDOR));
            };
            let name = opts
                .get("name")
                .cloned()
                .unwrap_or_else(|| format!("vendor {org}"));
            let buy = opts
                .get("buy")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(300);
            match crate::api::register_vendor(
                ctx,
                &instrument,
                org,
                vault,
                market,
                SmolStr::new(name),
                buy,
            ) {
                Ok(id) => {
                    info(ctx, format!("Vendor #{id} registered."));
                    Ok(())
                }
                Err(e) => Err(refuse_market(ctx, &e)),
            }
        }
        "fund" => {
            require_staff(ctx)?;
            let Some(vendor) = target.first().and_then(|s| parse_u64(s)).map(VendorId) else {
                return Err(CmdError::msg(
                    "Usage: @market/vendor fund <vendor>=<amount> faucet=<id> [period=<secs>]",
                ));
            };
            let Some(amount) = rhs
                .split_whitespace()
                .next()
                .and_then(|s| s.parse::<i64>().ok())
                .map(Amount::from_minor)
            else {
                return Err(CmdError::msg("Name an amount."));
            };
            let Some(faucet) = opts
                .get("faucet")
                .and_then(mush_org::SystemPrincipalId::new)
            else {
                return Err(CmdError::msg(
                    "Name the registered faucet the drip comes from (faucet=<id>). There is no \
                     other route: a vendor is funded by capitalization, by sales, or by an \
                     attributed drip under the [economy] mint cap.",
                ));
            };
            let period = opts
                .get("period")
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(86_400);
            let Some(credit) = ctx
                .w
                .module_state::<MarketStore>()
                .and_then(|s| s.vendor(vendor).map(|v| v.account))
            else {
                return Err(refuse(ctx, NO_SUCH_VENDOR));
            };
            let faucet_account = ctx
                .w
                .module_state::<mush_economy::EconomyStore>()
                .and_then(|s| {
                    s.snapshot()
                        .accounts_of(&mush_org::PrincipalId::System(faucet.clone()))
                        .next()
                        .map(|a| a.id)
                });
            let Some(faucet_account) = faucet_account else {
                return Err(CmdError::msg("That faucet has no account on this server."));
            };
            match crate::api::fund_from_faucet(
                ctx,
                &faucet,
                faucet_account,
                credit,
                amount,
                period,
                "market.funding",
            ) {
                Ok(id) => {
                    info(ctx, format!("Funding obligation #{id} created."));
                    Ok(())
                }
                Err(e) => Err(refuse_market(ctx, &e)),
            }
        }
        "stock" => {
            let Some(vendor) = target.first().and_then(|s| parse_u64(s)).map(VendorId) else {
                return Err(CmdError::msg(
                    "Usage: @market/vendor stock <vendor>=<ruleset>:<code> price=<n> [qty=<n>] …",
                ));
            };
            let Some(catalog) = rhs.split_whitespace().next().and_then(parse_catalog) else {
                return Err(CmdError::msg("Name what is being stocked."));
            };
            // E7r2 P1: an organization's own officer may stock it. `list_goods` still
            // refuses a non-lot representation to a non-staff actor below, so the
            // title-minting arm keeps the authority it always had.
            if !may_stock_for(ctx, vendor) {
                return Err(refuse(ctx, NO_SUCH_VENDOR));
            }
            let staff = api::is_staff(ctx.w, ctx.actor);
            let Some(price) = opts
                .get("price")
                .and_then(|s| s.parse::<i64>().ok())
                .map(Amount::from_minor)
            else {
                return Err(CmdError::msg("Name a price."));
            };
            let qty = opts
                .get("qty")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(1);
            let object = opts
                .get("object")
                .and_then(|s| mush_core::match_object(ctx.w, ctx.actor, s).ok());
            let vault = ctx
                .w
                .module_state::<MarketStore>()
                .and_then(|s| s.vendor(vendor).map(|v| v.vault));
            let Some(vault) = vault else {
                return Err(refuse(ctx, NO_SUCH_VENDOR));
            };
            let now = ctx.w.now();
            // ---- E7r2: P1 — `endpoint=<vault>` selects the lot supply ----
            //
            // The commodity defaults to the catalogue **code**, which is the mapping
            // the corridor pack already uses (`tacsim:iron` sells `iron`), and
            // `commodity=` overrides it for a pack that names them differently. The
            // grade is the listing's advertisement and rides onto the buyer's lot.
            let lot_endpoint = opts
                .get("endpoint")
                .and_then(|s| s.parse::<u64>().ok())
                .map(|id| mush_inventory::EndpointRef(mush_economy::VaultId(id)));
            let source = match lot_endpoint {
                Some(endpoint) => {
                    let raw = opts
                        .get("commodity")
                        .map_or_else(|| catalog.code.to_string(), ToString::to_string);
                    let Ok(commodity) = mush_core::CanonicalId::new(&raw) else {
                        return Err(CmdError::msg("That is not a commodity id."));
                    };
                    let grade = opts
                        .get("grade")
                        .and_then(|s| s.parse::<u8>().ok())
                        .unwrap_or(0);
                    if grade > mush_inventory::MAX_GRADE {
                        return Err(CmdError::msg(format!(
                            "A grade is 0 to {}.",
                            mush_inventory::MAX_GRADE
                        )));
                    }
                    Supply::Lot {
                        endpoint: crate::record::LotEndpoint(endpoint),
                        commodity: crate::record::LotCommodity(commodity),
                        grade,
                    }
                }
                None => match opts.get("lead").and_then(|s| s.parse::<i64>().ok()) {
                    Some(lead) => Supply::Procurement {
                        lead: mush_economy::EconomicDeadline {
                            clock: mush_economy::EconomicClock::RealUtc,
                            at: now.saturating_add(lead.max(60)),
                        },
                    },
                    None => match opts.get("source").and_then(|s| parse_u64(s)) {
                        Some(n) => Supply::Market {
                            source: MarketSourceId(n),
                        },
                        None => Supply::Stock { vault },
                    },
                },
            };
            // Everything but the lot supply stays a staff act: `Supply::Stock`,
            // `Supply::Market` and `Supply::Procurement` advertise a counter against
            // the shop's own balance sheet, and the `Asset` arm MINTS TITLE as a side
            // effect of publishing.
            if !staff && !matches!(source, Supply::Lot { .. }) {
                return Err(CmdError::msg("Permission denied."));
            }
            let days = opts
                .get("days")
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(14);
            // Doc 18 cut IC0: a lot listing stands no longer than the hold behind it
            // (`RESERVATION_TTL_IC_DAYS`); `api::list_goods` cuts the real-time window
            // to the hold's at the world's rate, and this is the day count's own cap.
            let days = if matches!(source, Supply::Lot { .. }) {
                days.min(mush_inventory::RESERVATION_TTL_IC_DAYS)
            } else {
                days
            };
            match crate::api::list_goods(
                ctx,
                crate::api::ListingSpec {
                    vendor,
                    catalog,
                    price,
                    quantity: qty,
                    source,
                    object,
                    visibility: if opts.contains_key("hidden") {
                        MarketVisibility::Hidden
                    } else {
                        MarketVisibility::Public
                    },
                    ttl_secs: days.saturating_mul(86_400),
                },
            ) {
                Ok(id) => {
                    info(ctx, format!("Listing #{id} published."));
                    Ok(())
                }
                Err(e) => Err(refuse_market(ctx, &e)),
            }
        }
        // ---- E7r2: P1 — the release path, as a SWITCH on an existing verb ----
        //
        // `api::withdraw_listing` has existed since E6c with no command behind it, and
        // a lot listing's hold needs one: a lapse is the sweep's, but a seller changing
        // their mind is an act. It is a new **sub-verb**, not a new primary verb, so
        // the pinned allowlist gains no stanza and `PINNED_BASELINE` does not move.
        "withdraw" => {
            let Some(listing) = target.first().and_then(|s| parse_u64(s)).map(ListingId) else {
                return Err(CmdError::msg("Usage: @market/vendor withdraw <listing>"));
            };
            // Existence-hiding: a listing this actor may not see and one that does not
            // exist read the same line, through the same predicate `+market/show` uses.
            let visible = ctx
                .w
                .module_state::<MarketStore>()
                .and_then(|s| s.listing(listing).cloned())
                .filter(|l| may_view_listing(ctx.w, ctx.actor, l));
            let Some(l) = visible else {
                return Err(refuse(ctx, NO_SUCH_LISTING));
            };
            if !may_stock_for(ctx, l.vendor) {
                return Err(refuse(ctx, NO_SUCH_LISTING));
            }
            match crate::api::withdraw_listing(ctx, listing) {
                Ok(()) => {
                    info(
                        ctx,
                        format!(
                            "Listing #{listing} withdrawn. Any inventory it held is released \
                             back to its endpoint."
                        ),
                    );
                    Ok(())
                }
                Err(e) => Err(refuse_market(ctx, &e)),
            }
        }
        "retire" => {
            require_staff(ctx)?;
            let Some(vendor) = target.first().and_then(|s| parse_u64(s)).map(VendorId) else {
                return Err(CmdError::msg("Usage: @market/vendor retire <vendor>"));
            };
            match crate::api::retire_vendor(ctx, vendor) {
                Ok(n) => {
                    info(
                        ctx,
                        format!(
                            "Vendor #{vendor} retired; {n} listing(s) withdrawn. Its live \
                             contracts still settle."
                        ),
                    );
                    Ok(())
                }
                Err(e) => Err(refuse_market(ctx, &e)),
            }
        }
        _ => Err(CmdError::msg(
            "Usage: @market/vendor create|fund|stock|retire …",
        )),
    }
}

fn admin_source(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let lhs = ctx.args.lhs().trim().to_string();
    let rhs = ctx.args.rhs().unwrap_or_default().trim().to_string();
    let mut words = lhs.split_whitespace();
    let verb = words.next().unwrap_or("").to_ascii_lowercase();
    let target: Vec<&str> = words.collect();
    let opts = parse_opts(&rhs);
    match verb.as_str() {
        "create" => {
            let Some(org) = target
                .first()
                .and_then(|s| parse_u64(s))
                .map(mush_org::OrgId)
            else {
                return Err(CmdError::msg(
                    "Usage: @market/source create <org>=<vault> [market=] [spread=] [volume=] \
                     [loss=] [window=]",
                ));
            };
            let Some(vault) = rhs
                .split_whitespace()
                .next()
                .and_then(parse_u64)
                .map(mush_economy::VaultId)
            else {
                return Err(CmdError::msg("Name the maker's inventory vault."));
            };
            let Some(market) = opts
                .get("market")
                .and_then(|m| MarketId::new(m))
                .or_else(|| MarketId::new("default"))
            else {
                return Err(CmdError::msg("That is not a venue name."));
            };
            let instrument = ctx
                .w
                .module_state::<mush_economy::EconomyStore>()
                .and_then(|s| {
                    s.snapshot()
                        .accounts_of(&mush_org::PrincipalId::Organization(org))
                        .next()
                        .map(|a| a.instrument.clone())
                });
            let Some(instrument) = instrument else {
                return Err(refuse(ctx, api::NO_SUCH_SOURCE));
            };
            let num =
                |k: &str, d: i64| opts.get(k).and_then(|s| s.parse::<i64>().ok()).unwrap_or(d);
            match crate::api::create_source(
                ctx,
                &instrument,
                org,
                vault,
                market,
                num("spread", 0),
                Amount::from_minor(num("volume", 0)),
                Amount::from_minor(num("loss", 0)),
                num("window", 86_400),
            ) {
                Ok(id) => {
                    info(ctx, format!("Market maker #{id} registered."));
                    Ok(())
                }
                Err(e) => Err(refuse_market(ctx, &e)),
            }
        }
        "budget" => {
            let Some(id) = target
                .first()
                .and_then(|s| parse_u64(s))
                .map(MarketSourceId)
            else {
                return Err(CmdError::msg(
                    "Usage: @market/source budget <source>=volume=<n> loss=<n> spread=<n>",
                ));
            };
            let a = |k: &str| {
                opts.get(k)
                    .and_then(|s| s.parse::<i64>().ok())
                    .map(Amount::from_minor)
            };
            match crate::api::budget_source(
                ctx,
                id,
                a("volume"),
                a("loss"),
                opts.get("spread").and_then(|s| s.parse::<i64>().ok()),
            ) {
                Ok(()) => {
                    info(ctx, format!("Market maker #{id} re-budgeted."));
                    Ok(())
                }
                Err(e) => Err(refuse_market(ctx, &e)),
            }
        }
        "halt" => {
            let Some(id) = target
                .first()
                .and_then(|s| parse_u64(s))
                .map(MarketSourceId)
            else {
                return Err(CmdError::msg(
                    "Usage: @market/source halt <source> [on|off]",
                ));
            };
            let on = !target.get(1).is_some_and(|w| w.eq_ignore_ascii_case("off"));
            match crate::api::halt_source(ctx, id, on) {
                Ok(()) => {
                    info(
                        ctx,
                        format!(
                            "Market maker #{id} {}. Its outstanding quotes settle nothing.",
                            if on { "halted" } else { "resumed" }
                        ),
                    );
                    Ok(())
                }
                Err(e) => Err(refuse_market(ctx, &e)),
            }
        }
        _ => Err(CmdError::msg("Usage: @market/source create|budget|halt …")),
    }
}

/// Whether a word is a usable pack **name** — the one `check_entry` already applies to
/// `CatalogEntry.pack`, asked before the name reaches a filesystem join (D9).
///
/// Non-empty, at most `record::MAX_PACK_BYTES` long, and `[A-Za-z0-9._-]` throughout —
/// which excludes `/`, `\` and every traversal by construction rather than by
/// enumerating them.
fn is_pack_name(pack: &str) -> bool {
    !pack.is_empty()
        && pack.len() <= crate::record::MAX_PACK_BYTES
        && pack
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

/// `key=value` options out of the right-hand side — the shape every E-series verb
/// takes. Bounded: at most sixteen, so a pathological line costs nothing.
fn parse_opts(s: &str) -> std::collections::BTreeMap<String, String> {
    s.split_whitespace()
        .filter_map(|w| {
            w.split_once('=')
                .map(|(k, v)| (k.to_ascii_lowercase(), v.to_string()))
                .or_else(|| Some((w.to_ascii_lowercase(), String::new())))
        })
        .take(16)
        .collect()
}

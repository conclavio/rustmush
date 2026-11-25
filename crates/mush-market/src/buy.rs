//! **`+market/buy`: buying a good by name in one step**, and the refusals that point the way
//! (the 2026-11-24 dev1 playtest: a new captain could not buy fuel for the first launch).
//!
//! `+market/buy <good> <amount>` finds the good by name on the board where the buyer stands,
//! quotes the cheapest ask that has that much, and says the price, the total and the one command
//! that takes it up. `+market/buy go` takes up the buyer's newest open quote. Both go through the
//! same [`crate::trade::quote`] and [`crate::trade::accept_at`] the two-step `+market/quote` and
//! `+market/accept` use, so nothing about a sale differs by the verb that made it; the two-step
//! pair stays as it was for scripts and the bot.
//!
//! The board read is the viewer's own ([`api::visible`]), so a listing the buyer may not see is
//! never found by name either (existence hiding holds).

use mush_core::{CmdError, Ctx};
use mush_org::PrincipalId;

use crate::api::{self, MarketError, MarketFilter, refuse_market};
use crate::commands::{gate_venue, money, quote_instrument};
use crate::ids::{ListingId, QuoteId};
use crate::record::Listing;
use crate::store::{MARKET_PAGE_MAX, MarketStore};
use crate::trade::{QuoteRequest, QuoteWindow};

/// The verb's grammar.
pub const BUY_USAGE: &str = "To buy, type: +market/buy <good> <amount>  (for example: +market/buy \
     reaction mass 12). +market/list shows what is for sale here.";

/// `+market/buy go` with no open quote.
pub const BUY_NOTHING_WAITING: &str = "You have no quote waiting. To get one, type: +market/buy \
     <good> <amount>";

/// The hint after `No such quote.`: how a quote is got (the playtester typed the playthrough's
/// example quote number).
pub const QUOTE_HINT: &str = " A quote number comes from +market/quote <listing>=<amount> or \
     +market/buy <good> <amount>; +market/orders shows yours. To buy in one step, type: \
     +market/buy <good> <amount>";

/// The most board pages a name lookup reads (each up to [`MARKET_PAGE_MAX`] rows).
const MAX_PAGES: usize = 4;

/// The most good names a refusal lists.
const MAX_NAMES: usize = 12;

/// Every listing on the board `viewer` stands at (or every board, where no port names one),
/// as the viewer may see them.
fn board(ctx: &Ctx<'_>) -> Vec<Listing> {
    let filter = gate_venue(ctx).map(MarketFilter::Market);
    let Some(store) = ctx.w.module_state::<MarketStore>() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut after: Option<ListingId> = None;
    for _ in 0..MAX_PAGES {
        let page = api::visible(ctx.w, ctx.actor, filter.as_ref(), MARKET_PAGE_MAX, after);
        out.extend(page.ids.iter().filter_map(|id| store.listing(*id).cloned()));
        match page.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    out
}

/// A good's player-facing name: its catalog entry's name, else its code.
pub(crate) fn good_name(w: &mush_core::World, l: &Listing) -> String {
    w.module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&l.catalog).map(|e| e.name.to_string()))
        .unwrap_or_else(|| l.catalog.code.to_string())
}

/// `text` with its first letter capitalized, for a sentence that opens with a good's name.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().collect::<String>() + chars.as_str()
    })
}

/// Who sells (or bids for) a listing, for a sentence: the house or seller's name, else
/// "its seller".
fn who(w: &mush_core::World, l: &Listing) -> String {
    let name = crate::seller::seller_name(w, l);
    if name.is_empty() {
        "its seller".to_owned()
    } else {
        name
    }
}

/// Whether `actor` could buy from `l` at all: an ask with stock, not their own, and not a
/// private offer to someone else.
fn buyable(ctx: &Ctx<'_>, l: &Listing) -> bool {
    let me = PrincipalId::Object(ctx.actor);
    !l.source.is_wanted()
        && l.quantity > 0
        && l.private_to.as_ref().is_none_or(|to| *to == me)
        && !(l.is_personal() && mush_economy::acts_for(ctx.w, ctx.actor, &l.seller))
}

/// How well `query` names `l`'s good: 2 an exact name or code, 1 every word of the query in
/// the name, the code or a tag, 0 not at all.
fn name_match(w: &mush_core::World, l: &Listing, query: &str) -> u8 {
    let name = good_name(w, l).to_lowercase();
    let code = l.catalog.code.to_lowercase();
    if name == query || code == query {
        return 2;
    }
    let tags: Vec<String> = w
        .module_state::<MarketStore>()
        .and_then(|s| s.entry_of(&l.catalog))
        .map(|e| e.tags.iter().map(|t| t.to_lowercase()).collect())
        .unwrap_or_default();
    let all = query.split_whitespace().all(|word| {
        name.split_whitespace().any(|n| n.starts_with(word))
            || code.contains(word)
            || tags.iter().any(|t| t == word)
    });
    u8::from(all)
}

/// The distinct good names among `listings`, in board order, at most [`MAX_NAMES`].
fn names_of<'a>(w: &mush_core::World, listings: impl Iterator<Item = &'a Listing>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in listings {
        let name = good_name(w, l);
        if !out.contains(&name) {
            out.push(name);
        }
        if out.len() >= MAX_NAMES {
            break;
        }
    }
    out
}

/// `+market/buy …`.
pub(crate) fn buy(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let raw = ctx.args.raw().replace('=', " ");
    let raw = raw.trim();
    if raw.is_empty() {
        return Err(CmdError::msg(BUY_USAGE));
    }
    let mut words: Vec<&str> = raw.split_whitespace().collect();
    if words.first().is_some_and(|w| w.eq_ignore_ascii_case("go")) {
        // `+market/buy go [to=<vault>]`: where no port places the goods, the buyer names
        // the vault, as `+market/accept <quote> to=<vault>` does.
        let rest: Vec<&str> = words
            .iter()
            .skip(1)
            .copied()
            .filter(|w| !w.eq_ignore_ascii_case("to"))
            .collect();
        let destination = match rest.as_slice() {
            [] => None,
            [vault] => match vault.trim_start_matches('#').parse::<u64>() {
                Ok(id) => Some(mush_inventory::EndpointRef(mush_economy::VaultId(id))),
                Err(_) => return Err(CmdError::msg(BUY_USAGE)),
            },
            _ => return Err(CmdError::msg(BUY_USAGE)),
        };
        return buy_go(ctx, destination);
    }
    let amount = words
        .last()
        .and_then(|w| w.trim_start_matches('#').parse::<u64>().ok());
    if amount.is_some() {
        words.pop();
    }
    let query = words.join(" ").to_lowercase();
    if query.is_empty() {
        return Err(CmdError::msg(BUY_USAGE));
    }
    let listings = board(ctx);
    let scored: Vec<(u8, &Listing)> = listings
        .iter()
        .map(|l| (name_match(ctx.w, l, &query), l))
        .filter(|(score, _)| *score > 0)
        .collect();
    let best = scored.iter().map(|(score, _)| *score).max().unwrap_or(0);
    let matched: Vec<&Listing> = scored
        .iter()
        .filter(|(score, _)| *score == best)
        .map(|(_, l)| *l)
        .collect();
    let mut goods: Vec<&mush_economy::CatalogRef> = Vec::new();
    for l in &matched {
        if !goods.contains(&&l.catalog) {
            goods.push(&l.catalog);
        }
    }
    if goods.is_empty() {
        let for_sale = names_of(ctx.w, listings.iter().filter(|l| buyable(ctx, l)));
        let here = if for_sale.is_empty() {
            "Nothing is for sale here.".to_owned()
        } else {
            format!("For sale here: {}.", for_sale.join(", "))
        };
        return Err(CmdError::msg(format!(
            "Nothing called \"{query}\" is for sale here. {here} To buy, type: +market/buy <good> \
             <amount>"
        )));
    }
    if goods.len() > 1 {
        let names = names_of(ctx.w, matched.iter().copied());
        return Err(CmdError::msg(format!(
            "\"{query}\" could be {}. Name one, for example: +market/buy {} {}",
            names.join(" or "),
            names.first().map_or("", String::as_str),
            amount.unwrap_or(1)
        )));
    }
    let Some(first) = matched.first() else {
        return Err(CmdError::msg(BUY_USAGE));
    };
    let name = good_name(ctx.w, first);
    let asks: Vec<&Listing> = matched
        .iter()
        .copied()
        .filter(|l| buyable(ctx, l))
        .collect();
    if asks.is_empty() {
        // Only bids (or nothing in stock) for this good here.
        let bids: Vec<&Listing> = matched
            .iter()
            .copied()
            .filter(|l| l.source.is_wanted())
            .collect();
        let line = match bids.first() {
            Some(bid) => format!(
                "Nobody sells {name} here: {} buys it (listing #{}). To sell yours, type: \
                 +market/sell {}",
                who(ctx.w, bid),
                bid.id,
                bid.id
            ),
            None => format!("Nobody here has {name} for sale right now."),
        };
        return Err(CmdError::msg(line));
    }
    let store = ctx.w.module_state::<MarketStore>();
    let price_of = |l: &Listing| crate::trade::price_for(l, store).minor();
    let Some(amount) = amount else {
        let Some(cheapest) = asks.iter().min_by_key(|l| (price_of(l), l.id)).copied() else {
            return Err(CmdError::msg(BUY_USAGE));
        };
        let instrument = crate::commands::listing_instrument(ctx.w, cheapest);
        return Err(CmdError::msg(format!(
            "How much? {} sells {name} at {} each ({} for sale). To buy, type: +market/buy {name} \
             <amount>",
            who(ctx.w, cheapest),
            money(
                ctx.w,
                instrument.as_ref(),
                crate::trade::price_for(cheapest, store)
            ),
            cheapest.quantity
        )));
    };
    if amount == 0 {
        return Err(CmdError::msg(BUY_USAGE));
    }
    let fits = |l: &&Listing| crate::trade::sells_quantity(l, amount);
    let Some(pick) = asks
        .iter()
        .copied()
        .filter(fits)
        .min_by_key(|l| (price_of(l), l.id))
    else {
        let most = asks.iter().map(|l| l.quantity).max().unwrap_or(0);
        return Err(CmdError::msg(format!(
            "No one seller here has {amount} {name}: the most one has is {most}. Buy less, for \
             example: +market/buy {name} {most}"
        )));
    };
    let req = QuoteRequest {
        listing: pick.id,
        quantity: amount,
        window: QuoteWindow::Live,
        ttl_live: crate::trade::QUOTE_TTL_LIVE,
        ttl_remote: crate::trade::QUOTE_TTL_REMOTE,
        presence: crate::module::Presence::Here,
    };
    let seller = who(ctx.w, pick);
    match crate::trade::quote(ctx, &req) {
        Ok(id) => {
            let Some(q) = ctx
                .w
                .module_state::<MarketStore>()
                .and_then(|s| s.quote(id).cloned())
            else {
                crate::commands::info(ctx, format!("Quote #{id}. To buy it, type: +market/buy go"));
                return Ok(());
            };
            let instrument = quote_instrument(ctx.w, &q);
            let minutes = (q.expires.at - ctx.w.now()).max(60) / 60;
            let mut line = format!(
                "{seller} sells you {} {name} at {} each: {} in all (quote #{id}, good for {minutes} \
                 minutes; it holds nothing for you until you buy).",
                q.quantity,
                money(ctx.w, instrument.as_ref(), q.price),
                money(ctx.w, instrument.as_ref(), q.total()),
            );
            line.push_str(&crate::commands::venue_cut_lines(
                ctx,
                &q,
                instrument.as_ref(),
            ));
            line.push_str(" To buy it, type: +market/buy go");
            crate::commands::info(ctx, line);
            Ok(())
        }
        Err(e) => Err(refuse_with_way(ctx, &e, Some(pick.id))),
    }
}

/// `+market/buy go`: take up the buyer's newest open quote, delivered into `destination` when
/// one is named.
fn buy_go(
    ctx: &mut Ctx<'_>,
    destination: Option<mush_inventory::EndpointRef>,
) -> Result<(), CmdError> {
    let me = PrincipalId::Object(ctx.actor);
    let newest: Option<QuoteId> = ctx.w.module_state::<MarketStore>().and_then(|s| {
        s.quotes_of_buyer(&me)
            .filter(|q| q.contract.is_none() && !q.is_trade())
            .map(|q| q.id)
            .max()
    });
    let Some(id) = newest else {
        return Err(CmdError::msg(BUY_NOTHING_WAITING));
    };
    let fresh = ctx
        .w
        .module_state::<MarketStore>()
        .and_then(|s| s.quote(id).cloned())
        .is_some_and(|q| crate::trade::is_current(ctx, &q));
    if !fresh {
        return Err(CmdError::msg(format!(
            "{} To quote again, type: +market/buy <good> <amount>",
            crate::trade::STALE_QUOTE
        )));
    }
    crate::commands::take_up(ctx, id, destination)
}

/// Render a market refusal, with the way forward where the market knows one: a bid named
/// where an ask was wanted says what its house does buy and sell and who sells the good here
/// ([`bid_refusal`]); `No such quote.` says how a quote is got ([`QUOTE_HINT`]). Every other
/// refusal renders as [`refuse_market`] renders it. `listing` is the listing the act named,
/// when known.
pub(crate) fn refuse_with_way(
    ctx: &mut Ctx<'_>,
    e: &MarketError,
    listing: Option<ListingId>,
) -> CmdError {
    let rendered = refuse_market(ctx, e);
    match e {
        MarketError::Bad(why) if why == crate::trade::WANTED_IS_A_BID => listing
            .and_then(|id| bid_refusal(ctx, id))
            .map_or(rendered, CmdError::msg),
        MarketError::NoStanding(why) if *why == api::NO_SUCH_QUOTE => {
            CmdError::msg(format!("{}{QUOTE_HINT}", api::NO_SUCH_QUOTE))
        }
        _ => rendered,
    }
}

/// **A bid named where an ask was wanted**, in full: whose bid it is and for what, that its
/// house buys that good here and sells none of it, how to sell into it, who sells the good
/// here (if anyone does), and what the house itself sells. `None` when the listing is not a
/// bid the actor may see.
fn bid_refusal(ctx: &Ctx<'_>, id: ListingId) -> Option<String> {
    let store = ctx.w.module_state::<MarketStore>()?;
    let bid = store.listing(id)?.clone();
    if !bid.source.is_wanted() || !api::may_view_listing(ctx.w, ctx.actor, &bid) {
        return None;
    }
    let name = good_name(ctx.w, &bid);
    let house = who(ctx.w, &bid);
    let mut line = format!(
        "Listing #{id} is a bid: {house} buys {name} here and sells none of it. Nothing was \
         charged. (To sell {name} to them, type: +market/sell {id})"
    );
    let listings = board(ctx);
    let sellers: Vec<String> = {
        let mut out: Vec<String> = Vec::new();
        for l in listings
            .iter()
            .filter(|l| l.catalog == bid.catalog && buyable(ctx, l))
        {
            let seller = who(ctx.w, l);
            if !out.contains(&seller) {
                out.push(seller);
            }
        }
        out
    };
    if sellers.is_empty() {
        line.push_str(&format!(" Nobody here sells {name}."));
    } else {
        line.push_str(&format!(
            " {} is sold here by {}: +market/buy {name} <amount>",
            capitalized(&name),
            sellers.join(" and ")
        ));
    }
    let house_sells = names_of(
        ctx.w,
        listings
            .iter()
            .filter(|l| !l.is_personal() && l.vendor == bid.vendor && buyable(ctx, l)),
    );
    if !bid.is_personal() && !house_sells.is_empty() {
        line.push_str(&format!(" {house} sells {}.", house_sells.join(", ")));
    } else if !line.ends_with('.') {
        line.push('.');
    }
    line.push_str(" +market/list shows what is for sale.");
    Some(line)
}

/// **`+market/list`'s closing line**: how to buy, with the page's own numbers — the first ask
/// shown, by name and by number — and, when the page shows bids, what a bid is. `None` for a
/// page that shows nothing.
pub(crate) fn list_footer(ctx: &Ctx<'_>, shown: &[Listing]) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(ask) = shown.iter().find(|l| buyable(ctx, l)) {
        let name = good_name(ctx.w, ask);
        let example = ask.quantity.clamp(1, 10);
        let example = if ask.source.exact_lot().is_some() {
            ask.quantity
        } else {
            example
        };
        out.push(format!(
            "To buy, type: +market/buy <good> <amount>, for example +market/buy {name} {example} \
             (or quote it by number: +market/quote {}={example}, then +market/accept <quote>).",
            ask.id
        ));
    }
    if shown.iter().any(|l| l.source.is_wanted()) {
        out.push(
            "Rows marked \"wanted\" are bids: that house buys the good and sells none. To sell \
             into one, type: +market/sell <number>"
                .to_owned(),
        );
    }
    out
}

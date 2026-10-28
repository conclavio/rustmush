//! 17g C2c (C2 plan §2.2): the player market's commands. `+minds` reads the
//! current listings at a counter (bare, `/next <code>`) and one listing's
//! full terms (`<code>`) through the market's bounded command read
//! (`mush_market::mind_commands::MarketCommandReader`); `+minds/buy <code>`
//! asks for a quote, `/buy/pay` pays it into escrow, `/buy/complete` takes
//! the licence and `/buy/refund` asks for the payment back; `+minds/holdings`
//! lists the actor's own licences (`holdings`); `@mind/publish` lists a filed
//! mind the actor owns. Every submitting form wraps exactly one existing
//! entitlement legal action (`MarketCommandActions`), with parameters built
//! from what the reads returned, through `market_commands::submit`.
//! `+minds/next <code>` pages with the market's `ListingsAfter`, so a `MORE`
//! cursor that names a listing no longer current still pages on (the C2c fix
//! round, reviewer F4); a page can then be empty and still print `MORE`.
//!
//! `+minds` is registered by TacSim unconditionally (`arena_bridge`), beside
//! `@mind`. Without the command slot or its market read every form refuses
//! `NOT_OPEN`, as on a House-only launch, which has no slot. **The read is
//! granted on the format-2 Active launch since 17g C2e**: the lead's ruling
//! R1 raised the owner-read caps (48 then, 64 since the scoped passenger
//! facts), and `TacsimRuleset::register` requests `MarketCommandReader` over
//! the same composed manifest the entitlement holdings' grants take, in the
//! branch that registers the slot (33 services and 35 grants). So every
//! `+minds` form below and `@mind/publish` run through the real reader
//! there.
//!
//! **The actor's own purchase (17g C2g, the C2e plan §4.1 item 2),** over
//! the per-buyer entitlement index 17g C2f writes inside the money
//! transactions (`market_mind_entitlement_actors`), read through the market
//! read's own-only `OwnPurchase { code }`, which matches the code against
//! the actor's OWN indexed quotes and purchases and never the listing family
//! (`by_code`), so another actor's purchase reads exactly as none:
//! - `+minds <code>`: `DETAIL`, `TERMS_LINE` and `SPLIT` when the listing is
//!   current, else its refusal (`NO_LISTING`, `AMBIGUOUS`); then, in both
//!   cases, the own-purchase line when the actor has a record there
//!   (`YOUR_ESCROW` while funded, else `YOUR_QUOTE` with the price and the
//!   seconds left while the newest quote is live, `QUOTE_EXPIRED` after),
//!   and `ASK` only when the listing is current and the actor has none.
//! - `+minds/buy <code>` (at a counter): `QUOTING`, the Quote action (C2c's
//!   fix-round ruling C2C-3 held it until the own-purchase line existed).
//! - `+minds/buy/pay <code>` (at a counter): the actor's one personal
//!   account (`ACCOUNT`), then its newest LIVE quote there (`NO_QUOTE`
//!   otherwise); `PAYING` and the Fund action with that quote's token. An
//!   expired quote is `NO_QUOTE` here and never reaches the provider (17g
//!   C2g's brief, which outranks the C2c brief's "expired or not" and its
//!   stale-terms step through `/buy/pay`): `+minds <code>` shows it as
//!   `QUOTE_EXPIRED`, and the provider's own stale-quote refusal stays
//!   reachable through `+act`.
//! - `+minds/buy/complete <code>` (at a counter) and `/buy/refund <code>`
//!   (anywhere): the actor's funded purchase there (`NO_PURCHASE`
//!   otherwise); `COMPLETING` and the Grant, or `REFUNDING` and the Refund,
//!   with its operation token. Resolved through the actor's own index, so a
//!   listing withdrawn after Fund can still be completed or refunded.
//!
//! Each step wraps exactly one existing entitlement action with parameters
//! from the own-purchase read, never from the player (C2 plan §2.1), and its
//! idempotency subject is the quote or operation TOKEN, not the code
//! (`mind.buy.pay.<token>.<at>.<ordinal>`; `pay_key`, `settle_key`), and
//! `+minds/buy`'s is the listing's token and the token of the actor's most
//! recently quoted own record there, quote or funded purchase
//! (`buy_key`, `OwnPurchaseView::latest`): after a restart with a stopped
//! IC clock a step can meet an earlier key only as the same request, except
//! a `+minds/buy` after a Grant or Refund (`next_key`'s doc states the
//! residual).
//!
//! 17g C2d routes its forms from here: `+minds/renew` and its `/pay` and
//! `/decline` (`renew`, live since 17g C2h: the renewal steps read through
//! the market's own handles, which a TacSim command context cannot use, so
//! `renew` checks its own refusals and hands the step to the renewal
//! service's `front` continuation, which the service delegated to TacSim at
//! composition, the lead's ruling R3), and `+minds/resell`, `+minds/charter`
//! and their `/accept`s (`trade`, live since 17g C2g).

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::read_service::OwnerReadHandle;
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ModError, Registrar};
use mush_market::mind_commands::{
    CodeLookup, ListingForm, MarketCommandActions, MarketCommandReader, MarketCommandRequest,
    OwnPurchaseView, Protocol, PublicListingView, Step, is_code,
};
use mush_roll::commerce::CharterId;
use mush_roll::commerce::command_reader::{RollCommandReadRequest, RollCommandReadResult};
use mush_roll::commerce::holding_reader::{OriginalOwnerReader, OriginalReadRequest};
use mush_roll::commerce::rating::RatingClass;
use serde_json::json;

use super::author::number;
use super::words::{self, Outcome, Refusal, cut, fill, word};

/// `+minds`'s switches. The dispatch is by the exact switch set (`command`).
/// 17g C2d adds `renew`, `decline`, `resell`, `charter` and `accept`
/// (`pay` is shared with `/buy/pay`).
pub(crate) const SWITCHES: [&str; 11] = [
    "next", "holdings", "buy", "pay", "complete", "refund", "renew", "decline", "resell",
    "charter", "accept",
];
/// `+minds`'s `CommandSpec.help`, and the reply to any other switch set.
/// 17g C2d adds its last three lines.
pub(crate) const HELP: &str = "+minds [<code>]; +minds/next <code>; +minds/holdings [owner|paid <cursor>]\n+minds/buy <code>; +minds/buy/pay <code>; +minds/buy/complete <code>; +minds/buy/refund <code>\n+minds/renew <licence> (then /pay or /decline with the same licence)\n+minds/resell <licence>=<player> price=<credits>; +minds/resell/accept <licence> [price=<credits>]\n+minds/charter <filed>=<player> price=<credits>; +minds/charter/accept <filed> [price=<credits>]";
/// `@mind/publish`'s usage: exactly its `@mind` `HELP` line.
pub(crate) const USAGE_PUBLISH: &str = "@mind/publish <filed> form=sealed|open|open-resale [royalty=<per mille>] [transfer=no] (at a counter)";

/// A listing name's and a holdings name's width.
pub(crate) const LISTING_NAME: usize = 24;
pub(crate) const HOLDING_NAME: usize = 20;
const DAY: i64 = 86_400;

fn not_open() -> Refusal {
    Refusal::not_open()
}
fn help() -> Refusal {
    Refusal::usage(HELP)
}

/// Register `+minds` (C2 plan §2.2), under the caller's
/// `CommandCompatibility::Authoritative` class.
pub(crate) fn register(reg: &mut Registrar<'_>) -> Result<(), ModError> {
    reg.command(
        CommandSpec {
            name: "+minds".into(),
            shape: ArgShape::Raw,
            switches: SWITCHES.iter().map(|word| (*word).into()).collect(),
            help: HELP.into(),
            ..CommandSpec::default()
        },
        Arc::new(command),
    )
}

/// The dispatch table, by the exact switch set: `{}` lists (bare) or shows
/// one listing (a code); `{next}` pages; `{holdings}`; `{buy}`, and
/// `{buy, pay}`, `{buy, complete}`, `{buy, refund}`; 17g C2d's `{renew}`,
/// `{renew, pay}`, `{renew, decline}` (`renew`), `{resell}`,
/// `{resell, accept}`, `{charter}` and `{charter, accept}` (`trade`);
/// anything else is `HELP`. The set is sorted, so `/resell/accept` and
/// `/accept/resell` are one form.
fn command(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let switches: BTreeSet<String> = ctx.switches.iter().map(|s| s.to_string()).collect();
    let set: Vec<&str> = switches.iter().map(String::as_str).collect();
    let text = ctx.args.raw().trim().to_owned();
    match set.as_slice() {
        [] if text.is_empty() => list(ctx),
        [] => detail(ctx, &text),
        ["next"] => next(ctx, &text),
        ["holdings"] => super::holdings::holdings(ctx, &text),
        // 17g C2g: live beside the own-purchase line (module doc). 17g C3a:
        // parsed, then run (`typed` runs the same `run_*`).
        ["buy"] => staged(ctx, |ctx| run_quote(ctx, &parse_buy(&text)?)),
        ["buy", "pay"] => staged(ctx, |ctx| run_pay(ctx, &parse_buy(&text)?)),
        ["buy", "complete"] => staged(ctx, |ctx| {
            run_settle(ctx, &parse_buy(&text)?, Settle::Complete)
        }),
        ["buy", "refund"] => staged(ctx, |ctx| {
            run_settle(ctx, &parse_buy(&text)?, Settle::Refund)
        }),
        // 17g C2h: live through the renewal services' front (`renew`'s doc).
        ["renew"] => super::renew::renew(ctx, &text, super::renew::Renew::Status),
        ["pay", "renew"] => super::renew::renew(ctx, &text, super::renew::Renew::Pay),
        ["decline", "renew"] => super::renew::renew(ctx, &text, super::renew::Renew::Decline),
        // 17g C2g: live over the per-buyer offer indexes (`trade`'s doc).
        ["resell"] => super::trade::resell(ctx, &text),
        ["charter"] => super::trade::charter(ctx, &text),
        ["accept", "resell"] => super::trade::accept(ctx, &text, super::trade::Named::Resale),
        ["accept", "charter"] => super::trade::accept(ctx, &text, super::trade::Named::Charter),
        _ => Err(help().into()),
    }
}

/// 17g C3a: a line command's parse and run, its `Outcome` printed as C2
/// printed it (`market_commands::print`) and its refusal returned for the
/// dispatcher to print, with C2's bytes (`From<Refusal> for CmdError`).
pub(super) fn staged(
    ctx: &mut Ctx<'_>,
    run: impl FnOnce(&mut Ctx<'_>) -> Result<Outcome, Refusal>,
) -> Result<(), CmdError> {
    let outcome = run(ctx)?;
    super::print(ctx, outcome);
    Ok(())
}

/// A listing code as typed, or `HELP`.
fn code(text: &str) -> Result<&str, Refusal> {
    if is_code(text) { Ok(text) } else { Err(help()) }
}

/// The market and Roll reads, or `NOT_OPEN`.
pub(super) fn handles(
    ctx: &Ctx<'_>,
) -> Result<
    (
        OwnerReadHandle<MarketCommandReader>,
        OwnerReadHandle<OriginalOwnerReader>,
    ),
    Refusal,
> {
    let service = super::service(ctx)?;
    let market = service.market.clone().ok_or_else(not_open)?;
    Ok((market, service.roll.clone()))
}

/// The composed entitlement actions, or `NOT_OPEN`.
fn actions(ctx: &Ctx<'_>) -> Result<MarketCommandActions, Refusal> {
    ctx.reg
        .definitions()
        .map(MarketCommandActions::capture)
        .ok_or_else(not_open)
}

/// Whether the actor stands at an admitting counter (the market read's
/// `AtCounter`), or `NOT_OPEN` when the read cannot run. 17g C3a: the
/// `mind.market` panel asks it too (`query`).
pub(super) fn here(
    reads: &impl super::Reads,
    market: &OwnerReadHandle<MarketCommandReader>,
) -> Result<bool, Refusal> {
    reads
        .owner_read(market, MarketCommandRequest::at_counter())
        .map_err(|_| not_open())?
        .at_counter()
        .ok_or_else(not_open)
}

/// `COUNTER` unless the actor stands at an admitting counter.
fn at_counter(ctx: &Ctx<'_>, market: &OwnerReadHandle<MarketCommandReader>) -> Result<(), Refusal> {
    if here(ctx, market)? {
        Ok(())
    } else {
        Err(word!(COUNTER))
    }
}

/// The current listing `code` names: `NO_LISTING` for none (the read's
/// `Refused`), `AMBIGUOUS` for two current ones (17g C2e: `by_code` counts
/// only current listings). Any other failure of the read (or an
/// answer of another kind) is `NOT_OPEN`, as in `next`, so a read that could
/// not run never reads as "no listing has the code" (the C2e fix round,
/// reviewer tests/T1).
fn listing(
    ctx: &Ctx<'_>,
    market: &OwnerReadHandle<MarketCommandReader>,
    code: &str,
) -> Result<PublicListingView, Refusal> {
    let found = ctx
        .read_owner(market, MarketCommandRequest::listing_by_code(code))
        .map_err(|error| match error {
            mush_core::read_service::OwnerReadError::Refused => {
                word!(NO_LISTING, &[("code", code)])
            }
            _ => not_open(),
        })?
        .into_listing()
        .ok_or_else(not_open)?;
    match found {
        CodeLookup::One(view) => Ok(view),
        CodeLookup::Ambiguous => Err(word!(AMBIGUOUS, &[("code", code)])),
    }
}

/// The public Roll names of `charters` (at most 20), through the folded
/// original-holdings read; a charter Roll does not name reads as empty.
/// 17g C3a: through either context (`Reads`), so the `mind.market` panel
/// names its rows exactly as `+minds` does.
pub(super) fn names(
    reads: &impl super::Reads,
    roll: &OwnerReadHandle<OriginalOwnerReader>,
    charters: Vec<CharterId>,
) -> Result<Vec<(CharterId, String)>, Refusal> {
    if charters.is_empty() {
        return Ok(Vec::new());
    }
    let request = OriginalReadRequest::command(RollCommandReadRequest::charter_names(charters));
    let names = reads
        .owner_read(roll, request)
        .ok()
        .and_then(mush_roll::commerce::holding_reader::OriginalReadResult::into_command)
        .and_then(RollCommandReadResult::into_charter_names)
        .ok_or_else(not_open)?;
    Ok(names
        .into_iter()
        .map(|n| (n.charter(), n.name().to_owned()))
        .collect())
}
pub(super) fn name_of(names: &[(CharterId, String)], charter: CharterId) -> &str {
    names
        .iter()
        .find(|(id, _)| *id == charter)
        .map_or("", |(_, name)| name.as_str())
}

/// 17g C3a: one listings page through the market read, after the counter
/// check: the first page (`after` none: the read's `Refused` is `COUNTER`)
/// or the page after the listing the code `after` names, current or not
/// (`MarketCommandRequest::listings_after`; the counter is checked first, so
/// the read's `Refused` means no listing has the code: `NO_LISTING`). Any
/// other failure is `NOT_OPEN`. `+minds`, `+minds/next` and the `mind.market`
/// panel (`query`) page through this one function.
pub(super) fn listings_page(
    reads: &impl super::Reads,
    market: &OwnerReadHandle<MarketCommandReader>,
    after: Option<&str>,
) -> Result<mush_market::mind_commands::ListingsPage, Refusal> {
    let request = after.map_or_else(
        || MarketCommandRequest::listings(None),
        MarketCommandRequest::listings_after,
    );
    reads
        .owner_read(market, request)
        .map_err(|error| match (error, after) {
            (mush_core::read_service::OwnerReadError::Refused, None) => word!(COUNTER),
            (mush_core::read_service::OwnerReadError::Refused, Some(code)) => {
                word!(NO_LISTING, &[("code", code)])
            }
            _ => not_open(),
        })?
        .into_listings()
        .ok_or_else(not_open)
}

/// `+minds`: the first page of current listings.
fn list(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    let (market, roll) = handles(ctx)?;
    at_counter(ctx, &market)?;
    let page = listings_page(ctx, &market, None)?;
    show(ctx, &roll, &page)
}

/// `+minds/next <code>`: the page after the listing a `MORE` line named,
/// current or not (`MarketCommandRequest::listings_after`). The counter is
/// checked first, so the read's `Refused` means no listing has the code.
fn next(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let code = code(text)?;
    let (market, roll) = handles(ctx)?;
    at_counter(ctx, &market)?;
    let page = listings_page(ctx, &market, Some(code))?;
    show(ctx, &roll, &page)
}

/// One listings page: `EMPTY` at the end with nothing shown; else `HEAD`,
/// the rows and `FOOT` (when there are rows), and `MORE` when the page goes
/// on. A page the read cut short on listings no longer current has no rows
/// and still goes on: `HEAD` then `MORE`, never a false `EMPTY` (reviewer
/// F4).
fn show(
    ctx: &mut Ctx<'_>,
    roll: &OwnerReadHandle<OriginalOwnerReader>,
    page: &mush_market::mind_commands::ListingsPage,
) -> Result<(), CmdError> {
    let actor = ctx.actor;
    if page.rows().is_empty() && page.next().is_none() {
        ctx.out.info(actor, words::EMPTY.to_owned());
        return Ok(());
    }
    let names = names(
        ctx,
        roll,
        page.rows().iter().map(PublicListingView::charter).collect(),
    )?;
    for line in page_lines(page.rows(), page.next(), &names) {
        ctx.out.info(actor, line);
    }
    Ok(())
}

/// A page's lines (`show`) when it shows rows or goes on, from its rows, its
/// cursor and the charters' names.
pub(crate) fn page_lines(
    rows: &[PublicListingView],
    next: Option<mush_core::CommitToken>,
    names: &[(CharterId, String)],
) -> Vec<String> {
    let mut lines = vec![words::HEAD.to_owned()];
    for view in rows {
        lines.push(row_line(view, name_of(names, view.charter())));
    }
    if !rows.is_empty() {
        lines.push(words::FOOT.to_owned());
    }
    if let Some(next) = next {
        let code: String = next
            .to_string()
            .chars()
            .take(mush_market::mind_commands::CODE_CHARACTERS)
            .collect();
        lines.push(fill(words::MORE, &[("code", &code)]));
    }
    lines
}

/// `+minds <code>`: one listing's full terms, then the actor's own record
/// on it (module doc). It needs no counter. With no current listing and no
/// own record it refuses as before (`NO_LISTING` or `AMBIGUOUS`); with an
/// own record the refusal is printed and the own line follows it, so a
/// purchase funded before its listing was withdrawn still shows.
fn detail(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let code = code(text)?;
    let (market, roll) = handles(ctx)?;
    let found = listing(ctx, &market, code);
    if let Err(error) = &found
        && *error == not_open()
    {
        return Err(not_open().into());
    }
    let found = found.map_err(CmdError::from);
    let own = own_purchase(ctx, &market, code)?;
    let now = ctx.w.now();
    let own = own.and_then(|view| {
        own_line(
            code,
            view.funded().is_some(),
            view.quote().map(|q| (q.price_minor(), q.expires_at_utc())),
            now,
        )
    });
    let found = match found {
        Ok(view) => {
            let names = names(ctx, &roll, vec![view.charter()])?;
            Ok(detail_lines(&view, name_of(&names, view.charter())))
        }
        Err(refusal) => Err(refusal),
    };
    super::say(ctx, detail_reply(found, own, code)?)
}

/// 17g C2g: `+minds <code>`'s reply from the listing's lines (`DETAIL`,
/// `TERMS_LINE`, `SPLIT`) or its refusal, and the actor's own-purchase line
/// (`own_line`):
/// - a current listing: its lines, then the own line, or `ASK` when there is
///   none;
/// - no current listing and no own line: the refusal, returned (one line);
/// - no current listing and an own line (a purchase funded before its
///   listing was withdrawn, say): the refusal as an error line, then the own
///   line (`market_commands::refusal`).
pub(crate) fn detail_reply(
    found: Result<[String; 3], CmdError>,
    own: Option<String>,
    code: &str,
) -> Result<Vec<super::Said>, CmdError> {
    match found {
        Ok(lines) => {
            let last = own.unwrap_or_else(|| fill(words::ASK, &[("code", code)]));
            Ok(lines
                .into_iter()
                .chain(std::iter::once(last))
                .map(super::Said::Info)
                .collect())
        }
        Err(refusal) => match own {
            None => Err(refusal),
            Some(line) => super::refusal(vec![refusal.to_string(), line]),
        },
    }
}

/// 17g C2g: the own-purchase line for the actor's record on listing `code`
/// (C2c's words): `YOUR_ESCROW` while a purchase is funded; else, for the
/// newest quote (`price_minor`, `expires_at_utc`), `YOUR_QUOTE` with the
/// whole seconds left while `now` is before its expiry and `QUOTE_EXPIRED`
/// at or after it (the provider's own Fund test); `None` with neither.
pub(crate) fn own_line(
    code: &str,
    funded: bool,
    quote: Option<(i64, i64)>,
    now: i64,
) -> Option<String> {
    if funded {
        return Some(fill(words::YOUR_ESCROW, &[("code", code)]));
    }
    let (price_minor, expires_at_utc) = quote?;
    Some(if now < expires_at_utc {
        let seconds = expires_at_utc.saturating_sub(now).to_string();
        fill(
            words::YOUR_QUOTE,
            &[
                ("price", &crate::launch::credits(price_minor)),
                ("seconds", &seconds),
                ("code", code),
            ],
        )
    } else {
        fill(words::QUOTE_EXPIRED, &[("code", code)])
    })
}

/// 17g C2g: the actor's own record on the listing `code` names, through the
/// market read's `OwnPurchase` (the actor's own index only); `None` for no
/// record. A read that cannot run is `NOT_OPEN`.
fn own_purchase(
    ctx: &Ctx<'_>,
    market: &OwnerReadHandle<MarketCommandReader>,
    code: &str,
) -> Result<Option<OwnPurchaseView>, Refusal> {
    match ctx.read_owner(market, MarketCommandRequest::own_purchase(code)) {
        Ok(result) => result.into_own_purchase().ok_or_else(not_open),
        // Two of the actor's own listings sharing the code (the read's doc):
        // as no record.
        Err(mush_core::read_service::OwnerReadError::Refused) => Ok(None),
        Err(_) => Err(not_open()),
    }
}

/// `+minds/buy <code>` (at a counter): ask for a quote on that listing; no
/// money moves. The Quote offer's own provider rechecks the listing against
/// Roll's current facts and refuses one account's two characters. 17g C2g:
/// live (module doc); the key's subject is the listing's own token and the
/// token of the actor's most recently quoted own record there (`buy_key`),
/// read through the own-purchase kind just before the submission.
///
/// 17g C3a: `parse_buy` then `run_quote`; the typed `Quote { code }` runs
/// the same `run_quote` over the same selection (`typed`).
pub(super) fn run_quote(ctx: &mut Ctx<'_>, selection: &BuySelection) -> Result<Outcome, Refusal> {
    let code = selection.code.as_str();
    let (market, _) = handles(ctx)?;
    at_counter(ctx, &market)?;
    let view = listing(ctx, &market, code)?;
    let action = actions(ctx)?
        .entitlement(view.protocol(), Step::Quote)
        .ok_or_else(not_open)?;
    let latest = own_purchase(ctx, &market, code)?.map(|own| own.latest());
    let line = fill(words::QUOTING, &[("code", code)]);
    let (verb, subject) = buy_key(view.publication(), latest);
    super::stage(
        ctx,
        verb,
        &subject,
        action,
        quote_parameters(view.publication()),
        line,
    )
}

/// 17g C3a: what the four buy steps select, a listing code as `+minds`
/// prints it (`parse_buy`; the typed `ListingCode`, `typed`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct BuySelection {
    pub(super) code: String,
}

/// 17g C3a: `+minds/buy <code>`'s and its three steps' argument: one
/// listing code (`is_code`), else the `+minds` help (`REFUSED`).
pub(super) fn parse_buy(text: &str) -> Result<BuySelection, Refusal> {
    Ok(BuySelection {
        code: code(text)?.to_owned(),
    })
}

/// 17g C2g: `+minds/buy`'s idempotency verb and subject (`next_key`): the
/// listing's token, then, when the actor has an own record there, the token
/// of its most recently quoted one, a quote or a funded purchase
/// (`OwnPurchaseView::latest`; the C2g fix round 2, reviewers
/// correctness/C2G-R2-1, conformance/F1 and tests/T-1). Every Quote and every
/// Fund the actor commits there changes the subject to one the listing never
/// had before, so after a restart under a stopped IC clock a re-quote never
/// meets the key of a quote request before it while those records stand;
/// with nothing committed between them, two requests are the same request.
/// The residual (`next_key`'s doc): a Grant or Refund takes a funded
/// purchase out of the actor's index, and the subject can be an earlier
/// one's again (the bare listing once no record is left).
pub(crate) fn buy_key(
    listing: mush_core::CommitToken,
    latest: Option<mush_core::CommitToken>,
) -> (&'static str, String) {
    let subject = match latest {
        None => listing.to_string(),
        Some(record) => format!("{listing}.{record}"),
    };
    ("buy", subject)
}

/// 17g C2g: `/buy/pay`'s idempotency verb and subject: the quote's token.
pub(crate) fn pay_key(quote: mush_core::CommitToken) -> (&'static str, String) {
    ("buy.pay", quote.to_string())
}

/// 17g C2g: `/buy/complete`'s and `/buy/refund`'s idempotency verb and
/// subject: the funded purchase's operation token.
pub(crate) fn settle_key(
    which: Settle,
    operation: mush_core::CommitToken,
) -> (&'static str, String) {
    let verb = match which {
        Settle::Complete => "buy.complete",
        Settle::Refund => "buy.refund",
    };
    (verb, operation.to_string())
}

/// 17g C2g: `+minds/buy/pay <code>` (at a counter): the actor's one
/// personal account (`ACCOUNT`), its newest live quote on the listing
/// (`NO_QUOTE` for none, or for an expired one), then `PAYING` and the Fund
/// action with the quote's token. The Fund's wire is the quote alone: its
/// payer is the personal account the Quote's provider froze into the quote,
/// which the `ACCOUNT` check keeps unambiguous (C2 plan §7). An expired
/// quote is `NO_QUOTE` and never submitted (17g C2g's brief item 6, which
/// outranks the C2c brief's stale-terms path through `/buy/pay`: the C2h
/// story's stale step reads `NO_QUOTE` here and `QUOTE_EXPIRED` on
/// `+minds <code>`).
pub(super) fn run_pay(ctx: &mut Ctx<'_>, selection: &BuySelection) -> Result<Outcome, Refusal> {
    let code = selection.code.as_str();
    let (market, _) = handles(ctx)?;
    at_counter(ctx, &market)?;
    super::personal_account(ctx)?;
    let no_quote = || word!(NO_QUOTE, &[("code", code)]);
    let own = own_purchase(ctx, &market, code)?.ok_or_else(no_quote)?;
    let now = ctx.w.now();
    let quote = own
        .quote()
        .filter(|quote| now < quote.expires_at_utc())
        .ok_or_else(no_quote)?;
    let action = actions(ctx)?
        .entitlement(own.protocol(), Step::Fund)
        .ok_or_else(not_open)?;
    let line = fill(words::PAYING, &[("code", code)]);
    let (verb, subject) = pay_key(quote.quote());
    super::stage(
        ctx,
        verb,
        &subject,
        action,
        fund_parameters(quote.quote()),
        line,
    )
}

/// 17g C2g: which terminal step `settle` submits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Settle {
    /// `+minds/buy/complete`: the Grant, at a counter.
    Complete,
    /// `+minds/buy/refund`: the Refund, anywhere.
    Refund,
}

/// 17g C2g: `+minds/buy/complete <code>` (at a counter) and
/// `+minds/buy/refund <code>` (anywhere): the actor's funded purchase on the
/// listing (`NO_PURCHASE` otherwise), then `COMPLETING` and the Grant or
/// `REFUNDING` and the Refund, with its operation token.
pub(super) fn run_settle(
    ctx: &mut Ctx<'_>,
    selection: &BuySelection,
    which: Settle,
) -> Result<Outcome, Refusal> {
    let code = selection.code.as_str();
    let (market, _) = handles(ctx)?;
    if which == Settle::Complete {
        at_counter(ctx, &market)?;
    }
    let none = || word!(NO_PURCHASE, &[("code", code)]);
    let own = own_purchase(ctx, &market, code)?.ok_or_else(none)?;
    let operation = own.funded().ok_or_else(none)?;
    let (step, request) = match which {
        Settle::Complete => (Step::Grant, words::COMPLETING),
        Settle::Refund => (Step::Refund, words::REFUNDING),
    };
    let action = actions(ctx)?
        .entitlement(own.protocol(), step)
        .ok_or_else(not_open)?;
    let line = fill(request, &[("code", code)]);
    let (verb, subject) = settle_key(which, operation);
    super::stage(
        ctx,
        verb,
        &subject,
        action,
        terminal_parameters(operation),
        line,
    )
}

/// The Quote offer's wire: the listing's publication token.
pub(crate) fn quote_parameters(listing: mush_core::CommitToken) -> serde_json::Value {
    json!({"listing": listing})
}

/// 17g C2g: the Fund's wire (`mind_entitlement::provider`'s `Fund`): the
/// quote's token.
pub(crate) fn fund_parameters(quote: mush_core::CommitToken) -> serde_json::Value {
    json!({"quote": quote})
}

/// 17g C2g: the Grant's and Refund's wire (`mind_entitlement::provider`'s
/// `Terminal`): the funded purchase's operation token.
pub(crate) fn terminal_parameters(operation: mush_core::CommitToken) -> serde_json::Value {
    json!({"operation": operation})
}

// ---- rendering (the C2c brief's fills) ----

/// `sealed`, `open` or `open-resale`.
pub(crate) const fn form_word(form: ListingForm) -> &'static str {
    match form {
        ListingForm::Sealed => "sealed",
        ListingForm::Open => "open",
        ListingForm::OpenResale => "open-resale",
    }
}
pub(crate) const fn class_word(class: RatingClass) -> &'static str {
    match class {
        RatingClass::I => "I",
        RatingClass::II => "II",
        RatingClass::III => "III",
    }
}
/// A per-mille share as a percentage, with one decimal only when not whole.
pub(crate) fn percent(permille: u16) -> String {
    if permille.is_multiple_of(10) {
        (permille / 10).to_string()
    } else {
        format!("{}.{}", permille / 10, permille % 10)
    }
}
/// Whole IC days, rounded up.
pub(crate) fn days(seconds: i64) -> i64 {
    seconds.max(0).saturating_add(DAY - 1) / DAY
}
/// `ROW`'s flags: `no resale`, `derived`, both, or none.
pub(crate) fn row_flags(transferable: bool, derived: bool) -> &'static str {
    match (transferable, derived) {
        (true, false) => "",
        (false, false) => "no resale",
        (true, true) => "derived",
        (false, true) => "no resale, derived",
    }
}

/// What one listing line shows: the view's public facts and the charter's
/// name. The commands build it from a `PublicListingView` (`of`); the width
/// test builds it at the maxima.
#[derive(Clone, Debug)]
pub(crate) struct Shown<'a> {
    pub(crate) code: String,
    pub(crate) name: &'a str,
    pub(crate) form: ListingForm,
    pub(crate) class: RatingClass,
    pub(crate) price_minor: i64,
    pub(crate) term_seconds: i64,
    pub(crate) grace_seconds: i64,
    pub(crate) transferable: bool,
    pub(crate) derived: bool,
    pub(crate) levy_permille: u16,
    pub(crate) venue_permille: u16,
    pub(crate) parent_royalty_permille: u16,
}
impl<'a> Shown<'a> {
    pub(crate) fn of(view: &PublicListingView, name: &'a str) -> Self {
        Self {
            code: view.code(),
            name,
            form: view.form(),
            class: view.class(),
            price_minor: view.price_minor(),
            term_seconds: view.term_seconds(),
            grace_seconds: view.grace_seconds(),
            transferable: view.transferable(),
            derived: view.derived(),
            levy_permille: view.levy_permille(),
            venue_permille: view.venue_permille(),
            parent_royalty_permille: view.parent_royalty_permille(),
        }
    }
}

/// One `ROW` (trailing spaces trimmed).
pub(crate) fn row_line(view: &PublicListingView, name: &str) -> String {
    row_text(&Shown::of(view, name))
}
pub(crate) fn row_text(shown: &Shown<'_>) -> String {
    let name = cut(shown.name, LISTING_NAME);
    let price = crate::launch::credits(shown.price_minor);
    let term = days(shown.term_seconds).to_string();
    let grace = days(shown.grace_seconds).to_string();
    fill(
        words::ROW,
        &[
            ("code", &shown.code),
            ("name", &name),
            ("form", form_word(shown.form)),
            ("class", class_word(shown.class)),
            ("price", &price),
            ("term", &term),
            ("grace", &grace),
            ("flags", row_flags(shown.transferable, shown.derived)),
        ],
    )
    .trim_end()
    .to_owned()
}

/// `DETAIL`, `TERMS_LINE` and `SPLIT` for one listing.
pub(crate) fn detail_lines(view: &PublicListingView, name: &str) -> [String; 3] {
    detail_text(&Shown::of(view, name))
}
pub(crate) fn detail_text(shown: &Shown<'_>) -> [String; 3] {
    let name = cut(shown.name, LISTING_NAME);
    let derived = if shown.derived { ", derived" } else { "" };
    let term = days(shown.term_seconds).to_string();
    let grace = days(shown.grace_seconds).to_string();
    let resale = if shown.transferable {
        "can be resold"
    } else {
        "cannot be resold"
    };
    let price = crate::launch::credits(shown.price_minor);
    let levy = percent(shown.levy_permille);
    let venue = if shown.venue_permille == 0 {
        String::new()
    } else {
        format!(", venue {}%", percent(shown.venue_permille))
    };
    let parents = if shown.parent_royalty_permille == 0 {
        String::new()
    } else {
        format!(
            ", parent royalties {}%",
            percent(shown.parent_royalty_permille)
        )
    };
    [
        fill(
            words::DETAIL,
            &[
                ("code", &shown.code),
                ("name", &name),
                ("form", form_word(shown.form)),
                ("derived", derived),
                ("class", class_word(shown.class)),
            ],
        ),
        fill(
            words::TERMS_LINE,
            &[("term", &term), ("grace", &grace), ("resale", resale)],
        ),
        fill(
            words::SPLIT,
            &[
                ("price", &price),
                ("levy", &levy),
                ("venue", &venue),
                ("parents", &parents),
            ],
        ),
    ]
}

// ---- @mind/publish ----

/// `@mind/publish`'s parsed arguments.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PublishArguments {
    pub(crate) filed: u64,
    /// The List offer's `form` choice: `sealed`, `open` or `open_resale`.
    pub(crate) form: &'static str,
    /// As the reply prints it: `sealed`, `open` or `open-resale`.
    pub(crate) shown: &'static str,
    pub(crate) royalty: u16,
    pub(crate) transferable: bool,
    /// `price=` in minor units, `seats=` and `term=` in IC days, if given.
    pub(crate) price: Option<i64>,
    pub(crate) seats: Option<u64>,
    pub(crate) term: Option<u64>,
}
/// Why `@mind/publish`'s arguments do not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PublishRefusal {
    /// The usage line: a missing or malformed filed number, a missing form,
    /// an unknown or repeated key, or a malformed price, seats or term.
    Usage,
    Form,
    Royalty,
    Transfer,
}
impl PublishRefusal {
    const fn line(self) -> &'static str {
        match self {
            Self::Usage => USAGE_PUBLISH,
            Self::Form => words::BAD_FORM,
            Self::Royalty => words::BAD_ROYALTY,
            Self::Transfer => words::BAD_TRANSFER,
        }
    }
    /// 17g C3a: the refusal with its word (`line`'s text; the usage line is
    /// no word, `REFUSED`).
    pub(super) fn refusal(self) -> Refusal {
        let word = match self {
            Self::Usage => words::REFUSED,
            Self::Form => "BAD_FORM",
            Self::Royalty => "BAD_ROYALTY",
            Self::Transfer => "BAD_TRANSFER",
        };
        Refusal::of(word, self.line())
    }
}

/// `<filed> form=… [royalty=…] [transfer=yes|no] [price=…] [seats=…]
/// [term=…]`, strictly: each key once, nothing else.
pub(crate) fn publish_arguments(text: &str) -> Result<PublishArguments, PublishRefusal> {
    let mut words = text.split_whitespace();
    let filed = words.next().and_then(number).ok_or(PublishRefusal::Usage)?;
    let mut pairs = std::collections::BTreeMap::new();
    for word in words {
        let (key, value) = word.split_once('=').ok_or(PublishRefusal::Usage)?;
        if !matches!(
            key,
            "form" | "royalty" | "transfer" | "price" | "seats" | "term"
        ) || pairs.insert(key, value).is_some()
        {
            return Err(PublishRefusal::Usage);
        }
    }
    let (form, shown) = match pairs.get("form").copied().ok_or(PublishRefusal::Usage)? {
        "sealed" => ("sealed", "sealed"),
        "open" => ("open", "open"),
        "open-resale" => ("open_resale", "open-resale"),
        _ => return Err(PublishRefusal::Form),
    };
    let royalty = match pairs.get("royalty").copied() {
        None => 0,
        Some("0") => 0,
        Some(text) => number(text)
            .and_then(|n| u16::try_from(n).ok())
            .filter(|n| *n <= 300)
            .ok_or(PublishRefusal::Royalty)?,
    };
    if form == "sealed" && royalty != 0 {
        return Err(PublishRefusal::Royalty);
    }
    let transferable = match pairs.get("transfer").copied() {
        None | Some("yes") => true,
        Some("no") => false,
        Some(_) => return Err(PublishRefusal::Transfer),
    };
    // 17g C2e (C2e plan §2.1 item 4b): a zero is a value here, so that
    // `publish` refuses it with `TERMS` (which needs the counter's price and
    // term) rather than the usage line; `credits_minor`'s `1..=` range is the
    // trade price grammar and stays as it is.
    let price = pairs
        .get("price")
        .map(|text| publish_price(text).ok_or(PublishRefusal::Usage))
        .transpose()?;
    let seats = pairs
        .get("seats")
        .map(|text| count(text).ok_or(PublishRefusal::Usage))
        .transpose()?;
    let term = pairs
        .get("term")
        .map(|text| count(text).ok_or(PublishRefusal::Usage))
        .transpose()?;
    Ok(PublishArguments {
        filed,
        form,
        shown,
        royalty,
        transferable,
        price,
        seats,
        term,
    })
}

/// A credit amount as players type it (C2 plan §2.7): `<whole>[.<one or two
/// digits>]` with an optional `cr`, from 0.01 to 1,000,000, in minor units.
pub(crate) fn credits_minor(text: &str) -> Option<i64> {
    credits_amount(text).filter(|minor| (1..=100_000_000).contains(minor))
}

/// `@mind/publish`'s `price=` (17g C2e): the credit grammar of
/// `credits_minor`, zero included (`0`, `0cr`, `0.00`), so a zero parses and
/// `terms_match` refuses it. A publish-only parse.
fn publish_price(text: &str) -> Option<i64> {
    credits_amount(text).filter(|minor| (0..=100_000_000).contains(minor))
}

/// `@mind/publish`'s `seats=` and `term=` (17g C2e): a whole number as
/// `number` reads one, or exactly `0`, so a zero parses and `terms_match`
/// refuses it. A publish-only parse.
fn count(text: &str) -> Option<u64> {
    if text == "0" { Some(0) } else { number(text) }
}

/// The credit grammar's value in minor units, any size `i64` holds; the
/// callers bound it.
fn credits_amount(text: &str) -> Option<i64> {
    let text = text.strip_suffix("cr").unwrap_or(text);
    let (whole, cents) = match text.split_once('.') {
        Some((whole, cents)) => (whole, Some(cents)),
        None => (text, None),
    };
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    if !digits(whole) || (whole.len() > 1 && whole.starts_with('0')) || whole.len() > 7 {
        return None;
    }
    let cents = match cents {
        None => 0,
        Some(c) if digits(c) && c.len() <= 2 => {
            let n: i64 = c.parse().ok()?;
            if c.len() == 1 { n * 10 } else { n }
        }
        Some(_) => return None,
    };
    whole
        .parse::<i64>()
        .ok()?
        .checked_mul(100)?
        .checked_add(cents)
}

/// Whether the optional `price=`, `seats=` and `term=` are the market's own
/// current values: the policy price, one seat, the personal term in IC days.
pub(crate) fn terms_match(args: &PublishArguments, price_minor: i64, term_days: u64) -> bool {
    args.price.is_none_or(|p| p == price_minor)
        && args.seats.is_none_or(|s| s == 1)
        && args.term.is_none_or(|t| t == term_days)
}

/// The List offer's wire (`mind_entitlement::offers::List`): the owner
/// licence (a decimal string), the form choice, the royalty and the
/// author's transfer mark.
pub(crate) fn list_parameters(
    owner_licence: mush_roll::commerce::LicenceId,
    form: &str,
    royalty: u16,
    transferable: bool,
) -> serde_json::Value {
    json!({
        "owner_licence": owner_licence,
        "form": form,
        "royalty_permille": royalty,
        "transferable": transferable,
    })
}

/// The market's price in minor units and the personal term in IC days, from
/// the launch deployment's player market.
fn market_terms(ctx: &Ctx<'_>) -> Option<(i64, u64)> {
    if ctx
        .w
        .module_state_owner(std::any::TypeId::of::<
            crate::launch::registration::LaunchService,
        >())
        .map(|o| o.as_str())
        != Some(crate::NS)
    {
        return None;
    }
    let market = ctx
        .w
        .module_state::<crate::launch::registration::LaunchService>()?
        .0
        .deployment()
        .market
        .as_ref()?;
    let term = u64::try_from(crate::launch::personal_term_days()).ok()?;
    Some((market.terms.price_minor, term))
}

/// `PUBLISHING` (17g C2e, C2e plan §2.1 item 4c): the request line names the
/// listing row's frozen transferability and derivation as the row will show
/// them. `{flags}` is empty when `row_flags` is, else `; ` and its words, so
/// a transferable listing of an original line reads exactly as before 17g
/// C2e. The TacSim launch's personal policy is transferable policy-wide
/// (D-UM-40, `launch::market`), so the row's frozen mark is the author's
/// `transfer=`; `derived` is the filed line's dependence, which picks the
/// derived protocol.
pub(crate) fn publishing_line(
    filed: &str,
    form: &str,
    transferable: bool,
    derived: bool,
) -> String {
    let flags = match row_flags(transferable, derived) {
        "" => String::new(),
        shown => format!("; {shown}"),
    };
    fill(
        words::PUBLISHING,
        &[("filed", filed), ("form", form), ("flags", &flags)],
    )
}

/// `@mind/publish <filed> form=…` (at a counter): list a filed mind the
/// actor owns through the original v3 List, or the derived v1 List when its
/// line is dependent, at the market's own price and terms with the author's
/// form, royalty and transfer mark. The caller (`WorkshopCommands::dispatch`)
/// has applied the workshop's principal rule.
pub(crate) fn publish(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    staged(ctx, |ctx| run_publish(ctx, &parse_publish(text)?))
}

/// 17g C3a: `@mind/publish`'s arguments (`publish_arguments`), refused with
/// their words (`PublishRefusal::refusal`).
pub(super) fn parse_publish(text: &str) -> Result<PublishArguments, Refusal> {
    publish_arguments(text).map_err(PublishRefusal::refusal)
}

/// 17g C3a: `publish` after its parse, for the line and the typed `Publish`
/// alike (`typed` builds the same `PublishArguments`).
pub(super) fn run_publish(ctx: &mut Ctx<'_>, args: &PublishArguments) -> Result<Outcome, Refusal> {
    let (market, _) = handles(ctx)?;
    at_counter(ctx, &market)?;
    let (price, term) = market_terms(ctx).ok_or_else(not_open)?;
    if !terms_match(args, price, term) {
        return Err(word!(
            TERMS,
            &[
                ("price", &crate::launch::credits(price)),
                ("term", &term.to_string()),
            ]
        ));
    }
    let view = super::author::own_filed(ctx, args.filed)?;
    let request = list_request(args, view.dependent(), view.owner_licence());
    let action = actions(ctx)?
        .entitlement(request.protocol, Step::List)
        .ok_or_else(not_open)?;
    let filed = args.filed.to_string();
    super::stage(
        ctx,
        "publish",
        &filed,
        action,
        request.parameters,
        request.line,
    )
}

/// What `publish` submits for a filed mind it owns: the List protocol, the
/// offer's parameters and the `PUBLISHING` request line.
#[derive(Debug)]
pub(crate) struct ListRequest {
    pub(crate) protocol: Protocol,
    pub(crate) parameters: serde_json::Value,
    pub(crate) line: String,
}

/// `publish`'s request from the parsed arguments and the filed line's own
/// facts (`OwnFiledView::dependent` and `owner_licence`): a dependent line
/// lists through the derived v1 List, any other through the original v3
/// List; the offer carries the author's form, royalty and transfer mark; and
/// the request line names the same transfer mark and derivation as the
/// listing row will show them (edge (c)). One pure function builds all three
/// from the same two facts, so the C2e fix round's lib test (reviewer
/// tests/T2) pins the line `publish` prints against the offer it submits.
pub(crate) fn list_request(
    args: &PublishArguments,
    dependent: bool,
    owner_licence: mush_roll::commerce::LicenceId,
) -> ListRequest {
    let protocol = if dependent {
        Protocol::Derived
    } else {
        Protocol::Original
    };
    ListRequest {
        protocol,
        parameters: list_parameters(owner_licence, args.form, args.royalty, args.transferable),
        line: publishing_line(
            &args.filed.to_string(),
            args.shown,
            args.transferable,
            dependent,
        ),
    }
}

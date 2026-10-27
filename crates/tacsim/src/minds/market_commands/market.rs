//! 17g C2c (C2 plan §2.2): the player market's commands. `+minds` reads the
//! current listings at a counter (bare, `/next <code>`) and one listing's
//! full terms (`<code>`) through the market's bounded command read
//! (`mush_market::mind_commands::MarketCommandReader`); `+minds/buy <code>`
//! asks for a quote; `+minds/holdings` lists the actor's own licences
//! (`holdings`); `@mind/publish` lists a filed mind the actor owns. Every
//! submitting form wraps exactly one existing entitlement legal action
//! (`MarketCommandActions`), with parameters built from what the reads
//! returned, through `market_commands::submit`. `+minds/next <code>` pages
//! with the market's `ListingsAfter`, so a `MORE` cursor that names a
//! listing no longer current still pages on (the C2c fix round, reviewer
//! F4); a page can then be empty and still print `MORE`.
//!
//! `+minds` is registered by TacSim unconditionally (`arena_bridge`), beside
//! `@mind`. Without the command slot or its market read every form refuses
//! `NOT_OPEN`: a House-only launch has no slot, and **no composition grants
//! the market read yet**: the format-2 Active launch already holds all 32
//! owner-read grants (17g C2a's measurement), so TacSim's
//! `MarketCommandReader::request` would refuse the boot with
//! `OwnerReadError::Limit`. The C2c brief says to stop there and leave the cap
//! to the lead (C2 plan §2.4 fallback 2), so `MindCommandService::market` is
//! `None` in every composition until that lands.
//!
//! **Not built (the C2c builder's report):** `+minds/buy/pay`,
//! `/buy/complete` and `/buy/refund`, and the actor's own quote or escrow
//! line under `+minds <code>`, read the actor's own quote and funded
//! operation, which needs the per-buyer entitlement index C2b did not land
//! (C2 plan §2.5 item 1). Their switches are registered and refuse
//! `NOT_OPEN` until that index lands; their words wait in
//! `words::pending`. **`+minds/buy` is held with them** (the C2c fix round,
//! reviewer C2C-3): its `QUOTING` reply promises that `+minds <code>` shows
//! the quote, which only the own-purchase line can, and a quote no command
//! can pay only expires. `buy` below is built and waits for that index.
//!
//! 17g C2d routes its forms from here: `+minds/renew` and its `/pay` and
//! `/decline` (`renew`), and `+minds/resell`, `+minds/charter` and their
//! `/accept`s (`trade`). Each is held as its module says: the renewal steps
//! read through the market's own handles, which a TacSim command context
//! cannot use (the C2d fix round, reviewer C2D-R1), and the trades wait for
//! the per-buyer offer indexes. So on the format-2 launch every C2d `+minds`
//! form runs its own checks and then refuses `NOT_OPEN`.

use std::collections::BTreeSet;
use std::sync::Arc;

use mush_core::read_service::OwnerReadHandle;
use mush_core::{ArgShape, CmdError, CommandSpec, Ctx, ModError, Registrar};
use mush_market::mind_commands::{
    CodeLookup, ListingForm, MarketCommandActions, MarketCommandReader, MarketCommandRequest,
    Protocol, PublicListingView, Step, is_code,
};
use mush_roll::commerce::CharterId;
use mush_roll::commerce::command_reader::{RollCommandReadRequest, RollCommandReadResult};
use mush_roll::commerce::holding_reader::{OriginalOwnerReader, OriginalReadRequest};
use mush_roll::commerce::rating::RatingClass;
use serde_json::json;

use super::author::number;
use super::words::{self, cut, fill};

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

fn not_open() -> CmdError {
    CmdError::msg(words::NOT_OPEN)
}
fn help() -> CmdError {
    CmdError::msg(HELP)
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
        [] if text.is_empty() => list(ctx, None),
        [] => detail(ctx, &text),
        ["next"] => next(ctx, &text),
        ["holdings"] => super::holdings::holdings(ctx, &text),
        // The per-buyer index these need is not landed (module doc); `/buy`
        // waits with them, so its reply never promises a quote line.
        ["buy"] | ["buy", "pay"] | ["buy", "complete"] | ["buy", "refund"] => {
            code(&text)?;
            handles(ctx)?;
            Err(not_open())
        }
        // 17g C2d: held at `renew::RENEWALS_OPEN` and
        // `trade::NAMED_OFFERS_OPEN` (each module's doc).
        ["renew"] => super::renew::renew(ctx, &text, super::renew::Renew::Status),
        ["pay", "renew"] => super::renew::renew(ctx, &text, super::renew::Renew::Pay),
        ["decline", "renew"] => super::renew::renew(ctx, &text, super::renew::Renew::Decline),
        ["resell"] => super::trade::resell(ctx, &text),
        ["charter"] => super::trade::charter(ctx, &text),
        ["accept", "charter"] | ["accept", "resell"] => super::trade::accept(ctx, &text),
        _ => Err(help()),
    }
}

/// A listing code as typed, or `HELP`.
fn code(text: &str) -> Result<&str, CmdError> {
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
    CmdError,
> {
    let service = super::service(ctx)?;
    let market = service.market.clone().ok_or_else(not_open)?;
    Ok((market, service.roll.clone()))
}

/// The composed entitlement actions, or `NOT_OPEN`.
fn actions(ctx: &Ctx<'_>) -> Result<MarketCommandActions, CmdError> {
    ctx.reg
        .definitions()
        .map(MarketCommandActions::capture)
        .ok_or_else(not_open)
}

/// `COUNTER` unless the actor stands at an admitting counter.
fn at_counter(
    ctx: &Ctx<'_>,
    market: &OwnerReadHandle<MarketCommandReader>,
) -> Result<(), CmdError> {
    let here = ctx
        .read_owner(market, MarketCommandRequest::at_counter())
        .map_err(|_| not_open())?
        .at_counter()
        .ok_or_else(not_open)?;
    if here {
        Ok(())
    } else {
        Err(CmdError::msg(words::COUNTER))
    }
}

/// The current listing `code` names: `NO_LISTING` for none, `AMBIGUOUS` for
/// two.
fn listing(
    ctx: &Ctx<'_>,
    market: &OwnerReadHandle<MarketCommandReader>,
    code: &str,
) -> Result<PublicListingView, CmdError> {
    let found = ctx
        .read_owner(market, MarketCommandRequest::listing_by_code(code))
        .ok()
        .and_then(mush_market::mind_commands::MarketCommandResult::into_listing);
    match found {
        Some(CodeLookup::One(view)) => Ok(view),
        Some(CodeLookup::Ambiguous) => {
            Err(CmdError::msg(fill(words::AMBIGUOUS, &[("code", code)])))
        }
        None => Err(CmdError::msg(fill(words::NO_LISTING, &[("code", code)]))),
    }
}

/// The public Roll names of `charters` (at most 20), through the folded
/// original-holdings read; a charter Roll does not name reads as empty.
fn names(
    ctx: &Ctx<'_>,
    roll: &OwnerReadHandle<OriginalOwnerReader>,
    charters: Vec<CharterId>,
) -> Result<Vec<(CharterId, String)>, CmdError> {
    if charters.is_empty() {
        return Ok(Vec::new());
    }
    let request = OriginalReadRequest::command(RollCommandReadRequest::charter_names(charters));
    let names = ctx
        .read_owner(roll, request)
        .ok()
        .and_then(mush_roll::commerce::holding_reader::OriginalReadResult::into_command)
        .and_then(RollCommandReadResult::into_charter_names)
        .ok_or_else(not_open)?;
    Ok(names
        .into_iter()
        .map(|n| (n.charter(), n.name().to_owned()))
        .collect())
}
fn name_of(names: &[(CharterId, String)], charter: CharterId) -> &str {
    names
        .iter()
        .find(|(id, _)| *id == charter)
        .map_or("", |(_, name)| name.as_str())
}

/// `+minds`: the first page of current listings.
fn list(ctx: &mut Ctx<'_>, after: Option<mush_core::CommitToken>) -> Result<(), CmdError> {
    let (market, roll) = handles(ctx)?;
    at_counter(ctx, &market)?;
    let page = ctx
        .read_owner(&market, MarketCommandRequest::listings(after))
        .map_err(|error| match error {
            mush_core::read_service::OwnerReadError::Refused => CmdError::msg(words::COUNTER),
            _ => not_open(),
        })?
        .into_listings()
        .ok_or_else(not_open)?;
    show(ctx, &roll, &page)
}

/// `+minds/next <code>`: the page after the listing a `MORE` line named,
/// current or not (`MarketCommandRequest::listings_after`). The counter is
/// checked first, so the read's `Refused` means no listing has the code.
fn next(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let code = code(text)?;
    let (market, roll) = handles(ctx)?;
    at_counter(ctx, &market)?;
    let page = ctx
        .read_owner(&market, MarketCommandRequest::listings_after(code))
        .map_err(|error| match error {
            mush_core::read_service::OwnerReadError::Refused => {
                CmdError::msg(fill(words::NO_LISTING, &[("code", code)]))
            }
            _ => not_open(),
        })?
        .into_listings()
        .ok_or_else(not_open)?;
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

/// `+minds <code>`: one listing's full terms. It needs no counter.
fn detail(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let code = code(text)?;
    let (market, roll) = handles(ctx)?;
    let view = listing(ctx, &market, code)?;
    let names = names(ctx, &roll, vec![view.charter()])?;
    let actor = ctx.actor;
    for line in detail_lines(&view, name_of(&names, view.charter())) {
        ctx.out.info(actor, line);
    }
    Ok(())
}

/// `+minds/buy <code>` (at a counter): ask for a quote on that listing; no
/// money moves. The Quote offer's own provider rechecks the listing against
/// Roll's current facts and refuses one account's two characters.
///
/// Held (module doc, reviewer C2C-3): `command` does not route here until
/// the per-buyer index lets `+minds <code>` show the quote `QUOTING`
/// promises and `+minds/buy/pay` pay it.
#[allow(dead_code)]
fn buy(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let code = code(text)?.to_owned();
    let (market, _) = handles(ctx)?;
    at_counter(ctx, &market)?;
    let view = listing(ctx, &market, &code)?;
    let action = actions(ctx)?
        .entitlement(view.protocol(), Step::Quote)
        .ok_or_else(not_open)?;
    let line = fill(words::QUOTING, &[("code", &code)]);
    super::submit(
        ctx,
        "quote",
        &code,
        action,
        quote_parameters(view.publication()),
        &line,
    )
}

/// The Quote offer's wire: the listing's publication token.
pub(crate) fn quote_parameters(listing: mush_core::CommitToken) -> serde_json::Value {
    json!({"listing": listing})
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
    let price = pairs
        .get("price")
        .map(|text| credits_minor(text).ok_or(PublishRefusal::Usage))
        .transpose()?;
    let seats = pairs
        .get("seats")
        .map(|text| number(text).ok_or(PublishRefusal::Usage))
        .transpose()?;
    let term = pairs
        .get("term")
        .map(|text| number(text).ok_or(PublishRefusal::Usage))
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
    let minor = whole
        .parse::<i64>()
        .ok()?
        .checked_mul(100)?
        .checked_add(cents)?;
    (1..=100_000_000).contains(&minor).then_some(minor)
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

/// `@mind/publish <filed> form=…` (at a counter): list a filed mind the
/// actor owns through the original v3 List, or the derived v1 List when its
/// line is dependent, at the market's own price and terms with the author's
/// form, royalty and transfer mark. The caller (`WorkshopCommands::dispatch`)
/// has applied the workshop's principal rule.
pub(crate) fn publish(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let args = publish_arguments(text).map_err(|refusal| CmdError::msg(refusal.line()))?;
    let (market, _) = handles(ctx)?;
    at_counter(ctx, &market)?;
    let (price, term) = market_terms(ctx).ok_or_else(not_open)?;
    if !terms_match(&args, price, term) {
        return Err(CmdError::msg(fill(
            words::TERMS,
            &[
                ("price", &crate::launch::credits(price)),
                ("term", &term.to_string()),
            ],
        )));
    }
    let view = super::author::own_filed(ctx, args.filed)?;
    let protocol = if view.dependent() {
        Protocol::Derived
    } else {
        Protocol::Original
    };
    let action = actions(ctx)?
        .entitlement(protocol, Step::List)
        .ok_or_else(not_open)?;
    let filed = args.filed.to_string();
    let line = fill(
        words::PUBLISHING,
        &[("filed", &filed), ("form", args.shown)],
    );
    let parameters = list_parameters(
        view.owner_licence(),
        args.form,
        args.royalty,
        args.transferable,
    );
    super::submit(ctx, "publish", &filed, action, parameters, &line)
}

//! 17g C3a (the C3 plan §2.3): the player market's two panel queries, each
//! the same name as its panel feed, so a client that subscribes the feed is
//! answered by its query (the TUI's auto-subscribe and `feed_query` identity
//! mapping depend on that). Both are registered by TacSim unconditionally
//! (`arena_bridge`), beside `+minds`, so the manifest is the same on every
//! launch format.
//!
//! - `mind.market` (`MarketPanel`): `AtCounter` first (away from a counter:
//!   `notice` = `COUNTER`, no rows); then one listings page, the first or the
//!   one after a code, through the very function `+minds` and
//!   `+minds/next <code>` page with (`market::listings_page`; a code no
//!   listing has is `notice` = `NO_LISTING` over the first page), each row
//!   named as `ROW` names it (`market::names`); and, at a counter or away,
//!   the actor's own quotes (the market read's `OwnQuotes`, 17g C3a) and
//!   waiting purchases (`OwnOpen`, `holdings::waiting`).
//! - `mind.holdings` (`HoldingsPanel`): one section of the actor's holdings,
//!   through the very functions `+minds/holdings owner|paid <cursor>` read
//!   (`holdings::owner_section`, `paid_section`), with the actions the
//!   server offers on each row (`owner_actions`, `paid_actions`; provisional,
//!   the C3 plan §7).
//!
//! Each reads the command slot at query time (`MindCommandService`, owner-
//! checked as the commands check it: `market_commands::slot`) and calls the
//! slot's two granted reads through the query's own read context, which
//! mush-core attaches as the dispatching actor and TacSim
//! (`Engine::handle_query`: `with_owner_read_caller`), so every read is the
//! actor's own and no account is resolved here. No slot, no market read, or
//! a read that cannot run (a format-1 launch) answers the closed keyframe:
//! `"open": false`, `notice` = `NOT_OPEN`, no rows.
//!
//! **The keyframes carry nothing the line does not print to the same actor**
//! (C2 plan §2.8, the C3 plan §2.6): every row field is a C2 view field the
//! line command shows; the listing token leaves only as its 8-character code
//! (the paging cursor is a code too); no row carries a `publication`,
//! `protocol`, `charter`, `owner_licence` or `renewal_operation`, and no
//! account or digest is ever serialized
//! (`market_panels_project_no_private_field`). The one record id a keyframe
//! carries is the owner section's paging cursor: `holdings::owner_section`'s
//! cursor is the last shown charter's asset id (a charter is its own E4
//! asset), exactly the number `+minds/holdings`' `HOLD_MORE` prints to the
//! same actor and the C3 plan §2.2 names ("an asset id for `owner`"); it
//! appears only as `after`, never in a row. Every
//! malformed request (an unknown key, a wrong section, a malformed code,
//! over 256 bytes) and a non-player or GOING actor answer one sentence,
//! `No such market page.`

use mush_core::module::{HookReadContext, QueryHandler};
use mush_core::{CommitToken, ObjId, World};
use mush_market::mind_commands::{MarketCommandRequest, OwnQuoteRowView, Protocol};
use mush_proto::{ListingCode, MindHoldingsSection, OutputEvent, PanelFrameKind, RichText};
use mush_roll::commerce::CharterId;
use mush_roll::commerce::command_reader::{
    OwnerHoldingView, OwnerPhase, PaidHoldingView, RenewalMark,
};
use mush_roll::commerce::paid::PaidForm;
use serde::Deserialize;
use serde_json::{Value, json};

use super::market::{HOLDING_NAME, LISTING_NAME, Shown};
use super::words::{self, Refusal, cut};

/// The listings panel's feed and query name.
pub(crate) const MARKET_FEED: &str = "mind.market";
/// The holdings panel's feed and query name.
pub(crate) const HOLDINGS_FEED: &str = "mind.holdings";
/// Both panels' schema version.
pub(crate) const SCHEMA_VERSION: u32 = 1;
/// The one refusal of every malformed request.
pub(crate) const NO_PAGE: &str = "No such market page.";
/// The longest params text either query reads (`mind_trade::query`'s bound).
const MAX_PARAMS_BYTES: usize = 256;
/// The longest keyframe either query answers.
const MAX_KEYFRAME_BYTES: usize = 32 * 1024;

/// `mind.market`'s params, decoded closed with the wire's own types (a
/// `ListingCode` refuses a malformed code before any read).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ListingsParams {
    pub(super) after: Option<ListingCode>,
}
/// `mind.holdings`'s params.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HoldingsParams {
    pub(super) section: MindHoldingsSection,
    pub(super) after: Option<u64>,
}

fn no_page() -> RichText {
    RichText::plain(NO_PAGE)
}

/// The params, bounded and decoded closed (a null params value reads as
/// `{}`), for a live, non-GOING player; else `No such market page.`
/// (`panel_params_refuse_malformed_requests_and_non_players`).
pub(super) fn params<T: serde::de::DeserializeOwned>(
    world: &World,
    actor: ObjId,
    params: &Value,
) -> Result<T, RichText> {
    if params.to_string().len() > MAX_PARAMS_BYTES {
        return Err(no_page());
    }
    let params = if params.is_null() {
        json!({})
    } else {
        params.clone()
    };
    let decoded = serde_json::from_value::<T>(params).map_err(|_| no_page())?;
    if world.get(actor).is_none_or(|o| {
        o.kind != mush_core::ObjKind::Player || o.flags.contains(mush_core::Flags::GOING)
    }) {
        return Err(no_page());
    }
    Ok(decoded)
}

/// One keyframe on `feed`, bounded.
fn keyframe(feed: &str, data: Value) -> Result<OutputEvent, RichText> {
    if data.to_string().len() > MAX_KEYFRAME_BYTES {
        return Err(no_page());
    }
    Ok(OutputEvent::Panel {
        feed: feed.into(),
        kind: PanelFrameKind::Keyframe,
        data,
    })
}

/// `{"id": feed, "version": 1}`.
fn schema(feed: &str) -> Value {
    json!({"id": feed, "version": SCHEMA_VERSION})
}

// ---- mind.market ----

/// 17g C3a: the `mind.market` query.
pub(crate) struct MarketPanel;
impl QueryHandler for MarketPanel {
    fn query(&self, _: &World, _: ObjId, _: &Value) -> Result<OutputEvent, RichText> {
        Err(no_page())
    }
    fn query_with_context(
        &self,
        ctx: &HookReadContext<'_>,
        actor: ObjId,
        args: &Value,
    ) -> Result<OutputEvent, RichText> {
        let request: ListingsParams = params(ctx.world(), actor, args)?;
        let data = match listings(ctx, request.after.as_ref()) {
            Ok(data) => data,
            Err(_) => closed_market(),
        };
        keyframe(MARKET_FEED, data)
    }
}

/// The open `mind.market` keyframe, or a `NOT_OPEN` refusal for the closed
/// one (no slot, no market read, or a read that cannot run).
fn listings(ctx: &HookReadContext<'_>, after: Option<&ListingCode>) -> Result<Value, Refusal> {
    let slot = super::slot(ctx.world())?;
    let market = slot.market.as_ref().ok_or_else(Refusal::not_open)?;
    // `AtCounter` first (the C3 plan §2.3); the own strip is read at a
    // counter or away.
    let here = super::market::here(ctx, market)?;
    let now = ctx.world().now();
    let quotes = ctx
        .read_owner(market, MarketCommandRequest::own_quotes())
        .ok()
        .and_then(mush_market::mind_commands::MarketCommandResult::into_own_quotes)
        .ok_or_else(Refusal::not_open)?;
    let waiting = super::holdings::waiting(ctx, market, None)?;
    let quotes: Vec<QuoteFacts> = quotes.iter().map(QuoteFacts::of).collect();
    let own = own_strip(&quotes, &waiting, now);
    if !here {
        return Ok(market_keyframe(true, Some(words::COUNTER), &[], None, own));
    }
    let (page, notice) = match super::market::listings_page(ctx, market, after.map(|c| c.as_str()))
    {
        Ok(page) => (page, None),
        Err(refusal) if refusal.word == "NO_LISTING" => (
            super::market::listings_page(ctx, market, None)?,
            Some(refusal.text),
        ),
        Err(refusal) => return Err(refusal),
    };
    let names = super::market::names(
        ctx,
        &slot.roll,
        page.rows()
            .iter()
            .map(mush_market::mind_commands::PublicListingView::charter)
            .collect(),
    )?;
    let rows: Vec<ListingFacts<'_>> = page
        .rows()
        .iter()
        .map(|view| ListingFacts::of(view, super::market::name_of(&names, view.charter())))
        .collect();
    let notice = notice
        .or_else(|| (rows.is_empty() && page.next().is_none()).then(|| words::EMPTY.to_owned()));
    Ok(market_keyframe(
        true,
        notice.as_deref(),
        &rows,
        page.next(),
        own,
    ))
}

/// 17g C3a: one listing as the panel reads it: every field of
/// `PublicListingView` (the token, protocol and charter included, which the
/// projection never serializes) and the charter's name.
#[derive(Clone, Debug)]
pub(super) struct ListingFacts<'a> {
    // Carried so the projection test plants it; never serialized (and never
    // read outside the tests).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) publication: CommitToken,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) protocol: Protocol,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) charter: CharterId,
    /// The public facts `ROW`, `DETAIL`, `TERMS_LINE` and `SPLIT` print.
    pub(super) shown: Shown<'a>,
}
impl<'a> ListingFacts<'a> {
    pub(super) fn of(view: &mush_market::mind_commands::PublicListingView, name: &'a str) -> Self {
        Self {
            publication: view.publication(),
            protocol: view.protocol(),
            charter: view.charter(),
            shown: Shown::of(view, name),
        }
    }
}

/// One `mind.market` row (the C3 plan §2.3): the listing's public facts as
/// `+minds` prints them, the credit rendering of the price (the line's
/// `{price}`), and `+minds <code>`'s `TERMS_LINE` and `SPLIT` lines, so a
/// client's confirm shows the price and the split without computing either.
/// The price in minor units is a decimal string (above 2^53 a JSON number is
/// not exact in a browser).
pub(super) fn listing_row(facts: &ListingFacts<'_>) -> Value {
    let shown = &facts.shown;
    let [_, terms_line, split_line] = super::market::detail_text(shown);
    json!({
        "code": shown.code,
        "name": cut(shown.name, LISTING_NAME),
        "class": super::market::class_word(shown.class),
        "form": super::market::form_word(shown.form),
        "transferable": shown.transferable,
        "term_seconds": shown.term_seconds,
        "grace_seconds": shown.grace_seconds,
        "price_minor": shown.price_minor.to_string(),
        "price_text": crate::launch::credits(shown.price_minor),
        "levy_permille": shown.levy_permille,
        "venue_permille": shown.venue_permille,
        "parent_royalty_permille": shown.parent_royalty_permille,
        "derived": shown.derived,
        "terms_line": terms_line,
        "split_line": split_line,
    })
}

/// 17g C3a: one of the actor's own quotes as the panel reads it (the market
/// read's `OwnQuoteRowView`, which carries no token).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct QuoteFacts {
    pub(super) code: String,
    pub(super) price_minor: i64,
    pub(super) expires_at_utc: i64,
}
impl QuoteFacts {
    pub(super) fn of(view: &OwnQuoteRowView) -> Self {
        Self {
            code: view.code().to_owned(),
            price_minor: view.price_minor(),
            expires_at_utc: view.expires_at_utc(),
        }
    }
}

/// The own strip: the actor's quotes (newest first, `OwnQuotes`), each with
/// its price, the whole seconds left and whether it has expired (the
/// provider refuses a Fund at or after its expiry), and its waiting
/// purchases' codes.
pub(super) fn own_strip(quotes: &[QuoteFacts], waiting: &[String], now: i64) -> Value {
    let quotes: Vec<Value> = quotes
        .iter()
        .map(|quote| {
            json!({
                "code": quote.code,
                "price_minor": quote.price_minor.to_string(),
                "price_text": crate::launch::credits(quote.price_minor),
                "expires_in_seconds": quote.expires_at_utc.saturating_sub(now).max(0),
                "expired": now >= quote.expires_at_utc,
            })
        })
        .collect();
    let waiting: Vec<Value> = waiting.iter().map(|code| json!({"code": code})).collect();
    json!({"quotes": quotes, "waiting": waiting})
}

/// The `mind.market` keyframe: `after` is the code `+minds`' `MORE` line
/// prints for the page's cursor (the last row's code while the page goes on;
/// the last listing scanned when the read's budget ended first), never the
/// token itself, and null at the end.
pub(super) fn market_keyframe(
    open: bool,
    notice: Option<&str>,
    rows: &[ListingFacts<'_>],
    next: Option<CommitToken>,
    own: Value,
) -> Value {
    let after = next.map(|token| {
        token
            .to_string()
            .chars()
            .take(mush_market::mind_commands::CODE_CHARACTERS)
            .collect::<String>()
    });
    json!({
        "schema": schema(MARKET_FEED),
        "open": open,
        "notice": notice,
        "rows": rows.iter().map(listing_row).collect::<Vec<_>>(),
        "after": after,
        "own": own,
    })
}

/// The closed `mind.market` keyframe (a format-1 launch).
pub(super) fn closed_market() -> Value {
    market_keyframe(
        false,
        Some(words::NOT_OPEN),
        &[],
        None,
        json!({"quotes": [], "waiting": []}),
    )
}

// ---- mind.holdings ----

/// 17g C3a: the `mind.holdings` query.
pub(crate) struct HoldingsPanel;
impl QueryHandler for HoldingsPanel {
    fn query(&self, _: &World, _: ObjId, _: &Value) -> Result<OutputEvent, RichText> {
        Err(no_page())
    }
    fn query_with_context(
        &self,
        ctx: &HookReadContext<'_>,
        actor: ObjId,
        args: &Value,
    ) -> Result<OutputEvent, RichText> {
        let request: HoldingsParams = params(ctx.world(), actor, args)?;
        let data = match holdings(ctx, request.section, request.after) {
            Ok(data) => data,
            Err(_) => holdings_keyframe(false, request.section, Vec::new(), None),
        };
        keyframe(HOLDINGS_FEED, data)
    }
}

/// The open `mind.holdings` keyframe for one section, or a `NOT_OPEN`
/// refusal for the closed one. Like every `+minds` form, it sits behind the
/// market read (`market::handles`).
fn holdings(
    ctx: &HookReadContext<'_>,
    section: MindHoldingsSection,
    after: Option<u64>,
) -> Result<Value, Refusal> {
    let slot = super::slot(ctx.world())?;
    slot.market.as_ref().ok_or_else(Refusal::not_open)?;
    let (rows, more) = match section {
        MindHoldingsSection::Owner => {
            let (rows, more) = super::holdings::owner_section(ctx, &slot.roll, after, None)?;
            (
                rows.iter()
                    .map(|view| owner_row(&OwnerFacts::of(view)))
                    .collect(),
                more,
            )
        }
        MindHoldingsSection::Paid => {
            let (rows, more) = super::holdings::paid_section(ctx, &slot.roll, after, None)?;
            (
                rows.iter()
                    .map(|view| paid_row(&PaidFacts::of(view)))
                    .collect(),
                more,
            )
        }
    };
    Ok(holdings_keyframe(true, section, rows, more))
}

/// The `mind.holdings` keyframe. `notice` is null when open (one section a
/// request: the client prints its own empty state) and `NOT_OPEN` when
/// closed.
pub(super) fn holdings_keyframe(
    open: bool,
    section: MindHoldingsSection,
    rows: Vec<Value>,
    after: Option<u64>,
) -> Value {
    json!({
        "schema": schema(HOLDINGS_FEED),
        "open": open,
        "notice": if open { None } else { Some(words::NOT_OPEN) },
        "section": section,
        "rows": rows,
        "after": after,
    })
}

/// `{"state": "active"|"grace"|"lapsed", "days_left": n|null}`.
fn phase(phase: OwnerPhase) -> Value {
    match phase {
        OwnerPhase::Active { days_left } => json!({"state": "active", "days_left": days_left}),
        OwnerPhase::Grace { days_left } => json!({"state": "grace", "days_left": days_left}),
        OwnerPhase::Lapsed => json!({"state": "lapsed", "days_left": null}),
    }
}

/// 17g C3a: one owner row as the panel reads it: every field of
/// `OwnerHoldingView` (the charter and owner licence included, which the
/// projection never serializes).
#[derive(Clone, Debug)]
pub(super) struct OwnerFacts {
    pub(super) filed: u64,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) charter: u64,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) owner_licence: u64,
    pub(super) dependent: bool,
    pub(super) name: String,
    pub(super) phase: OwnerPhase,
    pub(super) selling: bool,
}
impl OwnerFacts {
    pub(super) fn of(view: &OwnerHoldingView) -> Self {
        Self {
            filed: view.filed().get(),
            charter: u64::from(view.charter().get()),
            owner_licence: view.owner_licence().get(),
            dependent: view.dependent(),
            name: view.name().to_owned(),
            phase: view.owner_phase(),
            selling: view.selling(),
        }
    }
}

/// The actions the server offers on an owner row (provisional, the C3 plan
/// §7): `publish` and `withdraw` always, `charter` unless the charter is
/// being sold. One listed may still refuse at run (`COUNTER`); one not
/// listed is simply not offered.
pub(super) fn owner_actions(selling: bool) -> Vec<&'static str> {
    let mut actions = vec!["publish", "withdraw"];
    if !selling {
        actions.push("charter");
    }
    actions
}

/// One `mind.holdings` owner row: `HOLD_OWNER`'s facts (the name cut to the
/// row's 20 characters).
pub(super) fn owner_row(facts: &OwnerFacts) -> Value {
    json!({
        "kind": "owner",
        "filed": facts.filed,
        "name": cut(&facts.name, HOLDING_NAME),
        "dependent": facts.dependent,
        "phase": phase(facts.phase),
        "selling": facts.selling,
        "actions": owner_actions(facts.selling),
    })
}

/// 17g C3a: one paid row as the panel reads it: every field of
/// `PaidHoldingView` (the charter and the renewal's pending operation
/// included, which the projection never serializes).
#[derive(Clone, Debug)]
pub(super) struct PaidFacts {
    pub(super) licence: u64,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) charter: u64,
    pub(super) house: bool,
    pub(super) name: String,
    pub(super) form: PaidForm,
    pub(super) transferable: bool,
    pub(super) phase: OwnerPhase,
    pub(super) revoked: bool,
    pub(super) selling: bool,
    pub(super) renewal: Option<RenewalMark>,
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) renewal_operation: Option<CommitToken>,
}
impl PaidFacts {
    pub(super) fn of(view: &PaidHoldingView) -> Self {
        Self {
            licence: view.licence().get(),
            charter: u64::from(view.charter().get()),
            house: view.house(),
            name: view.name().to_owned(),
            form: view.form(),
            transferable: view.transferable(),
            phase: view.phase(),
            revoked: view.revoked(),
            selling: view.selling(),
            renewal: view.renewal(),
            renewal_operation: view.renewal_operation(),
        }
    }
}

/// The actions the server offers on a paid row (provisional, the C3 plan
/// §7): `renew` with no renewal, `renew_pay` and `renew_decline` while an
/// offer waits, none once it is paid; `resell` on a transferable, unrevoked
/// licence no resale holds. The renewal service may still answer
/// `RENEW_NOT_DUE`.
pub(super) fn paid_actions(facts: &PaidFacts) -> Vec<&'static str> {
    let mut actions = match facts.renewal {
        None => vec!["renew"],
        Some(RenewalMark::Offered) => vec!["renew_pay", "renew_decline"],
        Some(RenewalMark::Paid) => Vec::new(),
    };
    if facts.transferable && !facts.revoked && !facts.selling {
        actions.push("resell");
    }
    actions
}

/// One `mind.holdings` paid row: `HOLD_PAID`'s facts (the name cut to the
/// row's 20 characters, the form as `+minds/holdings` prints it).
pub(super) fn paid_row(facts: &PaidFacts) -> Value {
    let form = match facts.form {
        PaidForm::Sealed {} => "sealed",
        PaidForm::Open {} => "open",
        PaidForm::OpenResale {} => "open-resale",
    };
    let renewal = facts.renewal.map(|mark| match mark {
        RenewalMark::Offered => "offered",
        RenewalMark::Paid => "paid",
    });
    json!({
        "kind": "paid",
        "licence": facts.licence,
        "house": facts.house,
        "name": cut(&facts.name, HOLDING_NAME),
        "form": form,
        "transferable": facts.transferable,
        "phase": phase(facts.phase),
        "revoked": facts.revoked,
        "selling": facts.selling,
        "renewal": renewal,
        "actions": paid_actions(facts),
    })
}

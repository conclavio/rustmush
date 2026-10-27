//! 17g C2d (C2 plan §2.2, §2.6, §2.7): the title trades. A seller names one
//! buyer and a price; the named buyer sees the offers naming them and
//! confirms one by repeating its frozen price.
//!
//! - `+minds/resell <licence>=<player> price=<credits>`: a paid licence the
//!   actor holds (`OwnPaid`, else `NO_LICENCE`), offered to one player
//!   through the paid-resale Offer, `{"licence", "buyer", "price",
//!   "seller_account"}` (`mind_paid_resale::provider::OfferParameters`), the
//!   seller's own personal account receiving the proceeds. A licence from a
//!   `transfer=no` listing is refused by the provider (the driver's line).
//! - `+minds/charter <filed>=<player> price=<credits>`: a filed mind the
//!   actor owns (`OwnFiled`, else `NO_FILED`), its charter offered through
//!   the pending charter protocol's Offer (always the pending protocol, C2
//!   plan §2.6: correct whether or not sorties are out; the immediate
//!   `roll-offer-charter-transfer` stays on `+act`), `{"charter", "buyer",
//!   "seller_account", "price_minor", "valid_for_seconds"}` as decimal
//!   strings (`charter_pending::provider::OfferWire`), open 86,400 seconds
//!   (`charter_transfer::records::MAX_OFFER_SECONDS`).
//! - `+minds/resell/accept <licence> [price=<credits>]` and
//!   `+minds/charter/accept <filed> [price=<credits>]`: without `price=`, a
//!   line for each live offer naming the actor and how to accept it, and
//!   nothing is submitted; with `price=`, the live offer whose frozen price
//!   equals it (an equal-price tie takes the one expiring last), submitted
//!   as the resale's Accept or Fund (the provider's own open-claim test,
//!   through Roll's `ResaleStep` read, held with the accept: Roll's
//!   `command_reader` doc) or the pending charter's Fund, `{"offer",
//!   "payer"}`.
//!   No offer is `NO_OFFER` / `NO_CHARTER_OFFER`; no offer at that price is
//!   `NO_OFFER_AT` / `NO_CHARTER_OFFER_AT` and the listing (`accept_plan`).
//!
//! `<player>` resolves as `page` resolves one (`me`, `#<dbref>`, a name or
//! `*name`), and must be a live player character (`NO_PLAYER`). A seller
//! form checks, in order: its shape and price (the help, `BAD_PRICE`), the
//! command slot (`NOT_OPEN`), the player (`NO_PLAYER`, public: `page` and
//! `WHO` answer it too), then the actor's own record (`NO_LICENCE`,
//! `NO_FILED`). A price is
//! `<whole>[.<one or two digits>]` with an optional `cr`, from 0.01 to
//! 1,000,000 (`market::credits_minor`; `BAD_PRICE`). The seller's request
//! lines name the buyer the seller chose; a buyer's lines name no seller.
//!
//! **Held (the C2d builder's report).** A buyer finds the offers naming them
//! only through the per-(buyer, licence) and per-(buyer, charter) offer
//! indexes (C2 plan §2.5 items 3 and 4), which C2b did not land (held for
//! the lead's mush-core ruling), and the resale offers through the market's
//! command read's `ResaleOffers` kind, which waits for 17g C2g over those
//! indexes (the read itself is granted since 17g C2e).
//! So both accepts parse and then refuse `NOT_OPEN`, and their listing and
//! price rule (`accept_plan`) wait, built and tested. By C2c's rule for
//! `+minds/buy` (reviewer C2C-3: never make an offer no command can take),
//! the two seller forms run every check of their own and then refuse
//! `NOT_OPEN` at `NAMED_OFFERS_OPEN` instead of submitting; the index
//! sub-cut flips that one constant. Their parameter objects are proven
//! against the registered providers now (`hardware::flow::launch`'s
//! `trade_parameters_satisfy_the_registered_providers`).

use mush_core::{CmdError, CommitToken, Ctx, Flags, ObjId, ObjKind};
use mush_market::mind_commands::{MarketCommandActions, ResaleKind};
use mush_roll::commerce::command_actions::RollCommandActions;
use mush_roll::commerce::{CharterId, LicenceId};
use serde_json::json;

use super::market::{HELP, credits_minor};
use super::words::{self, fill};

/// Whether a buyer can find the offers naming them, so a seller's offer can
/// be taken by command: false until the per-buyer offer indexes and the
/// market's command read land (module doc). The seller forms refuse
/// `NOT_OPEN` just before submitting while it is false.
pub(crate) const NAMED_OFFERS_OPEN: bool = false;

/// The pending charter offer's window, in seconds: the protocol's longest
/// (`charter_transfer::records::MAX_OFFER_SECONDS`), as the resale's.
pub(crate) const CHARTER_OFFER_SECONDS: i64 = 86_400;

fn not_open() -> CmdError {
    CmdError::msg(words::NOT_OPEN)
}

/// A seller's parsed `<subject>=<player> price=<credits>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TradeArguments {
    /// The licence or filed number.
    pub(crate) subject: u64,
    /// The buyer as typed.
    pub(crate) player: String,
    pub(crate) price_minor: i64,
}
/// A buyer's parsed `<subject> [price=<credits>]`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AcceptArguments {
    pub(crate) subject: u64,
    pub(crate) price_minor: Option<i64>,
}
/// Why trade arguments do not parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TradeRefusal {
    /// `+minds`'s help: a missing or malformed number, player or `price=`.
    Usage,
    /// `BAD_PRICE`: a `price=` outside the grammar or its bounds.
    Price,
}
impl TradeRefusal {
    pub(crate) fn error(self) -> CmdError {
        CmdError::msg(match self {
            Self::Usage => HELP,
            Self::Price => words::BAD_PRICE,
        })
    }
}

/// `price=<credits>`'s value in minor units, or `Price`.
fn price(text: &str) -> Result<i64, TradeRefusal> {
    credits_minor(text).ok_or(TradeRefusal::Price)
}

/// `<subject>=<player> price=<credits>`, strictly: a decimal number, `=`,
/// the player (a name may hold spaces, never `=`), one space and `price=`
/// last.
pub(crate) fn trade_arguments(text: &str) -> Result<TradeArguments, TradeRefusal> {
    let (subject, rest) = text.split_once('=').ok_or(TradeRefusal::Usage)?;
    let subject = super::author::number(subject.trim()).ok_or(TradeRefusal::Usage)?;
    let (player, priced) = rest.trim().rsplit_once(' ').ok_or(TradeRefusal::Usage)?;
    let player = player.trim();
    if player.is_empty() || player.contains('=') {
        return Err(TradeRefusal::Usage);
    }
    let value = priced.strip_prefix("price=").ok_or(TradeRefusal::Usage)?;
    Ok(TradeArguments {
        subject,
        player: player.to_owned(),
        price_minor: price(value)?,
    })
}

/// `<subject> [price=<credits>]`, strictly.
pub(crate) fn accept_arguments(text: &str) -> Result<AcceptArguments, TradeRefusal> {
    let mut words = text.split_whitespace();
    let subject = words
        .next()
        .and_then(super::author::number)
        .ok_or(TradeRefusal::Usage)?;
    let price_minor = match words.next() {
        None => None,
        Some(word) => Some(price(
            word.strip_prefix("price=").ok_or(TradeRefusal::Usage)?,
        )?),
    };
    if words.next().is_some() {
        return Err(TradeRefusal::Usage);
    }
    Ok(AcceptArguments {
        subject,
        price_minor,
    })
}

/// A price as `price=` spells it: `0.21`, `12.50`, `1000000`.
pub(crate) fn amount(minor: i64) -> String {
    let (whole, cents) = (minor.div_euclid(100), minor.rem_euclid(100));
    if cents == 0 {
        whole.to_string()
    } else {
        format!("{whole}.{cents:02}")
    }
}

/// The player `typed` names, as `page` resolves one, when it is a live
/// player character: its id and name. Else `NO_PLAYER`.
fn player(ctx: &Ctx<'_>, typed: &str) -> Result<(ObjId, String), CmdError> {
    let refuse = || CmdError::msg(fill(words::NO_PLAYER, &[("name", typed)]));
    let query = typed.trim();
    let id = if query.eq_ignore_ascii_case("me") {
        Some(ctx.actor)
    } else if let Some(number) = query.strip_prefix('#') {
        number.parse::<u32>().ok().map(ObjId)
    } else {
        ctx.w
            .resolve_player(query.strip_prefix('*').unwrap_or(query))
    };
    let object = id
        .and_then(|id| ctx.w.get(id).map(|o| (id, o)))
        .filter(|(_, o)| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING))
        .ok_or_else(refuse)?;
    Ok((object.0, object.1.name.clone()))
}

/// `+minds/resell <licence>=<player> price=<credits>`.
pub(crate) fn resell(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let args = trade_arguments(text).map_err(TradeRefusal::error)?;
    super::service(ctx)?;
    let (buyer, name) = player(ctx, &args.player)?;
    let held = super::renew::own_paid(ctx, args.subject)?;
    if !NAMED_OFFERS_OPEN {
        return Err(not_open());
    }
    let action = ctx
        .reg
        .definitions()
        .map(MarketCommandActions::capture)
        .and_then(|actions| actions.resale(ResaleKind::Offer))
        .ok_or_else(not_open)?;
    let account = super::personal_account(ctx)?;
    let licence = args.subject.to_string();
    let shown = crate::launch::credits(args.price_minor);
    let request = fill(
        words::RESELLING,
        &[("licence", &licence), ("player", &name), ("price", &shown)],
    );
    let parameters = resale_offer_parameters(held.licence(), buyer, args.price_minor, account);
    super::submit(ctx, "resell", &licence, action, parameters, &request)?;
    ctx.out.info(
        ctx.actor,
        fill(
            words::RESELL_TELL,
            &[("licence", &licence), ("amount", &amount(args.price_minor))],
        ),
    );
    Ok(())
}

/// `+minds/charter <filed>=<player> price=<credits>`.
pub(crate) fn charter(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let args = trade_arguments(text).map_err(TradeRefusal::error)?;
    super::service(ctx)?;
    let (buyer, name) = player(ctx, &args.player)?;
    let owned = super::author::own_filed(ctx, args.subject)?;
    if !NAMED_OFFERS_OPEN {
        return Err(not_open());
    }
    let action = ctx
        .reg
        .definitions()
        .map(RollCommandActions::capture)
        .and_then(|actions| actions.pending_charter_offer())
        .ok_or_else(not_open)?;
    let account = super::personal_account(ctx)?;
    let filed = args.subject.to_string();
    let shown = crate::launch::credits(args.price_minor);
    let request = fill(
        words::CHARTERING,
        &[("filed", &filed), ("player", &name), ("price", &shown)],
    );
    let parameters = charter_offer_parameters(owned.charter(), buyer, account, args.price_minor);
    super::submit(ctx, "charter", &filed, action, parameters, &request)?;
    ctx.out.info(
        ctx.actor,
        fill(
            words::CHARTER_TELL,
            &[("filed", &filed), ("amount", &amount(args.price_minor))],
        ),
    );
    Ok(())
}

/// `+minds/resell/accept <licence> [price=<credits>]` and
/// `+minds/charter/accept <filed> [price=<credits>]`: held (module doc).
pub(crate) fn accept(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    accept_arguments(text).map_err(TradeRefusal::error)?;
    super::service(ctx)?;
    Err(not_open())
}

/// The paid-resale Offer's wire (`OfferParameters`): numbers, not strings.
pub(crate) fn resale_offer_parameters(
    licence: LicenceId,
    buyer: ObjId,
    price_minor: i64,
    seller_account: mush_economy::AccountId,
) -> serde_json::Value {
    json!({
        "licence": licence.get(),
        "buyer": buyer.0,
        "price": price_minor,
        "seller_account": seller_account.0,
    })
}

/// The paid-resale Accept's and Fund's wire (`AcceptParameters`). Held with
/// the accepts (module doc).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn resale_accept_parameters(
    offer: CommitToken,
    payer: mush_economy::AccountId,
) -> serde_json::Value {
    json!({"offer": offer, "payer": payer.0})
}

/// The pending charter Offer's wire (`OfferWire`), every field a decimal
/// string.
pub(crate) fn charter_offer_parameters(
    charter: CharterId,
    buyer: ObjId,
    seller_account: mush_economy::AccountId,
    price_minor: i64,
) -> serde_json::Value {
    json!({
        "charter": charter.get().to_string(),
        "buyer": buyer.0.to_string(),
        "seller_account": seller_account.0.to_string(),
        "price_minor": price_minor.to_string(),
        "valid_for_seconds": CHARTER_OFFER_SECONDS.to_string(),
    })
}

/// The pending charter Fund's wire (`FundWire`): the offer token and the
/// payer as a decimal string. Held with the accepts (module doc).
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn charter_fund_parameters(
    offer: CommitToken,
    payer: mush_economy::AccountId,
) -> serde_json::Value {
    json!({"offer": offer, "payer": payer.0.to_string()})
}

// ---- the named buyer's listing and price rule (held: module doc) ----

/// One live offer naming the actor, as the offer indexes will answer it.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NamedOffer {
    pub(crate) offer: CommitToken,
    pub(crate) price_minor: i64,
    pub(crate) expires_at_utc: i64,
}

/// A resale offer (on a licence) or a charter offer (on a filed mind).
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Named {
    Resale,
    Charter,
}

/// What an accept does with the live offers naming the actor.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AcceptPlan {
    /// Print these lines and submit nothing (no `price=`, or a refusal).
    Show(Vec<String>),
    /// A refusal: these lines, the first the refusal sentence.
    Refuse(Vec<String>),
    /// Submit this offer, after this request line.
    Submit(NamedOffer, String),
}

/// 17g C2d (C2 plan §2.6): the accept's plan over `offers` (the live offers
/// naming the actor on `subject`) at UTC second `now`. An offer at or past
/// its expiry is not live. Shown in price order, then expiry order: an
/// offer line and its confirming line each.
/// - No live offer: `Refuse([NO_OFFER])` / `NO_CHARTER_OFFER`.
/// - No `price=`: `Show(the lines)`.
/// - `price=` matching a live offer's frozen price: `Submit` the one
///   expiring last among those at that price, with `ACCEPTING` /
///   `FUNDING_CHARTER`.
/// - Else `Refuse([NO_OFFER_AT / NO_CHARTER_OFFER_AT, the lines…])`.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn accept_plan(
    named: Named,
    subject: u64,
    offers: &[NamedOffer],
    price_minor: Option<i64>,
    now: i64,
) -> AcceptPlan {
    let subject = subject.to_string();
    let mut live: Vec<NamedOffer> = offers
        .iter()
        .copied()
        .filter(|o| o.expires_at_utc > now)
        .collect();
    live.sort_by_key(|o| (o.price_minor, o.expires_at_utc));
    let key = match named {
        Named::Resale => "licence",
        Named::Charter => "filed",
    };
    if live.is_empty() {
        let none = match named {
            Named::Resale => words::named::NO_OFFER,
            Named::Charter => words::named::NO_CHARTER_OFFER,
        };
        return AcceptPlan::Refuse(vec![fill(none, &[(key, &subject)])]);
    }
    let lines = offer_lines(named, &subject, &live, now);
    let Some(price_minor) = price_minor else {
        return AcceptPlan::Show(lines);
    };
    let price = crate::launch::credits(price_minor);
    match live
        .iter()
        .filter(|o| o.price_minor == price_minor)
        .max_by_key(|o| o.expires_at_utc)
    {
        Some(chosen) => {
            let request = match named {
                Named::Resale => fill(
                    words::named::ACCEPTING,
                    &[("licence", &subject), ("price", &price)],
                ),
                Named::Charter => fill(
                    words::named::FUNDING_CHARTER,
                    &[("price", &price), ("filed", &subject)],
                ),
            };
            AcceptPlan::Submit(*chosen, request)
        }
        None => {
            let none_at = match named {
                Named::Resale => words::named::NO_OFFER_AT,
                Named::Charter => words::named::NO_CHARTER_OFFER_AT,
            };
            let mut refused = vec![fill(none_at, &[(key, &subject), ("price", &price)])];
            refused.extend(lines);
            AcceptPlan::Refuse(refused)
        }
    }
}

/// Each live offer's line and its confirming line, in the order given.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn offer_lines(
    named: Named,
    subject: &str,
    live: &[NamedOffer],
    now: i64,
) -> Vec<String> {
    let (offer, how, key) = match named {
        Named::Resale => (words::named::OFFER, words::named::ACCEPT_HOW, "licence"),
        Named::Charter => (
            words::named::CHARTER_OFFER,
            words::named::CHARTER_ACCEPT_HOW,
            "filed",
        ),
    };
    let mut lines = Vec::with_capacity(live.len() * 2);
    for o in live {
        let price = crate::launch::credits(o.price_minor);
        let minutes = minutes_left(o.expires_at_utc, now).to_string();
        lines.push(fill(
            offer,
            &[(key, subject), ("price", &price), ("minutes", &minutes)],
        ));
        lines.push(fill(
            how,
            &[(key, subject), ("amount", &amount(o.price_minor))],
        ));
    }
    lines
}

/// Whole minutes from `now` to `until`, rounded up, never below 0.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn minutes_left(until: i64, now: i64) -> i64 {
    until.saturating_sub(now).max(0).saturating_add(59) / 60
}

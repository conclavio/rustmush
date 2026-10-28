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
//!   as the resale's Accept or Fund or the pending charter's Fund, `{"offer",
//!   "payer"}` with the actor's one personal account (`ACCOUNT`).
//!   No offer is `NO_OFFER` / `NO_CHARTER_OFFER`; no offer at that price is
//!   `NO_OFFER_AT` / `NO_CHARTER_OFFER_AT` and the listing (`accept_plan`).
//!
//! `<player>` resolves as `page` resolves one (`me`, `#<dbref>`, a name or
//! `*name`), and must be a live player character (`NO_PLAYER`). A seller
//! form checks, in order: its shape and price (the help, `BAD_PRICE`), the
//! command slot (`NOT_OPEN`), the player (`NO_PLAYER`, public: `page` and
//! `WHO` answer it too), then the actor's own record (`NO_LICENCE`,
//! `NO_FILED`), then that the player is not the actor itself (17g C2g: an
//! offer to oneself is `NO_PLAYER` with the name as typed, before any line
//! prints). The buyer the offer's parameters and request line name is the
//! one `other_player` hands back, so neither form can name a buyer that
//! check has not passed. The dispatched proof needs a held licence and an
//! owned filed mind, which the fast suites cannot make (the brief's
//! fixtures rule): owed to the C2h story's part 3 (a holder's
//! `+minds/resell <licence>=<own name> price=1` and an owner's
//! `+minds/charter <filed>=<own name> price=1` each read `No player is named
//! <own name>.`, with no request line and nothing submitted). The alt pair, two characters of one account, stays the
//! provider's refusal through the lobby's `AccountId` oracle (D-UM-40),
//! the driver's one line. A price is `<whole>[.<one or two digits>]` with an
//! optional `cr`, from 0.01 to 1,000,000 (`market::credits_minor`;
//! `BAD_PRICE`). The seller's request lines name the buyer the seller
//! chose; a buyer's lines name no seller.
//!
//! **The named offers (17g C2g, the C2e plan §4.1 item 4).** A buyer finds
//! the offers naming them only through 17g C2f's per-buyer offer indexes,
//! each written inside the money transaction it follows: the market read's
//! own-only `ResaleOffers { licence }` (the actor's `(buyer, licence)` row of
//! `market_mind_paid_resale_buyer_offers` and the offer rows it names) and
//! Roll's own-only `CharterOffers { filed }` (the actor's `(buyer, charter)`
//! row of `roll_mind_charter_pending_buyer_offers`). So `accept_plan` runs
//! over the offers whose COMMITTED buyer is the actor, with their frozen
//! price and expiry, and another buyer's offer is absent exactly as none
//! is. The resale's step, Accept or Fund, comes with each `ResaleOffers` view
//! (`ResaleOfferView::step`): the market read asks Roll's
//! `mush_roll::commerce::paid_resale::reader::resale_step` (the provider's
//! own claim test over Roll's live claim census) only for a licence on which
//! an offer naming the actor was found, so the step is decided beside the
//! offer's committed evidence and never here. The seller forms submit
//! (`RESELL_TELL` and `CHARTER_TELL` are the SELLER's second line: no line
//! reaches the named buyer, whose offers-to-you list is an open owner
//! question, 17g §5). Every
//! parameter object is proven against the registered providers
//! (`hardware::flow::launch`'s `trade_parameters_satisfy_the_registered_
//! providers`).

use mush_core::read_service::OwnerReadError;
use mush_core::{CmdError, CommitToken, Ctx, Flags, ObjId, ObjKind};
use mush_market::mind_commands::{MarketCommandActions, MarketCommandRequest, ResaleKind};
use mush_roll::commerce::command_actions::RollCommandActions;
use mush_roll::commerce::command_reader::{RollCommandReadRequest, RollCommandReadResult};
use mush_roll::commerce::holding_reader::OriginalReadRequest;
use mush_roll::commerce::{CharterId, FiledRevisionId, LicenceId};
use serde_json::json;

use super::market::{HELP, credits_minor};
use super::words::{self, Outcome, Refusal, fill, word};

/// The pending charter offer's window, in seconds: the protocol's longest
/// (`charter_transfer::records::MAX_OFFER_SECONDS`), as the resale's.
pub(crate) const CHARTER_OFFER_SECONDS: i64 = 86_400;

fn not_open() -> Refusal {
    Refusal::not_open()
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
    /// 17g C3a: the refusal with its word (`BAD_PRICE`; the help is no word,
    /// `REFUSED`), in place of C2's `error`, which printed the same text.
    pub(super) fn refusal(self) -> Refusal {
        match self {
            Self::Usage => Refusal::usage(HELP),
            Self::Price => word!(BAD_PRICE),
        }
    }
}

/// `price=<credits>`'s value in minor units, or `Price`.
pub(super) fn price(text: &str) -> Result<i64, TradeRefusal> {
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

/// 17g C3a: a seller form's selection (`trade_arguments`), refused with its
/// word: `+minds/resell` and `+minds/charter` take the same shape.
pub(super) fn parse_offer(text: &str) -> Result<TradeArguments, Refusal> {
    trade_arguments(text).map_err(TradeRefusal::refusal)
}

/// 17g C3a: an accept's selection (`accept_arguments`), refused with its
/// word.
pub(super) fn parse_accept(text: &str) -> Result<AcceptArguments, Refusal> {
    accept_arguments(text).map_err(TradeRefusal::refusal)
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
fn player(ctx: &Ctx<'_>, typed: &str) -> Result<(ObjId, String), Refusal> {
    let refuse = || no_player(typed);
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

/// 17g C2g: the seller's chosen buyer (`player`'s id and name), unless it is
/// the seller itself: an offer to oneself is `NO_PLAYER` with the name as
/// typed (checked after the seller's own record, so it never tells anyone
/// anything they did not know). The seller forms take the buyer they submit
/// from what this hands back (the C2g fix round 2, reviewer tests/T-4).
pub(crate) fn other_player(
    actor: ObjId,
    buyer: (ObjId, String),
    typed: &str,
) -> Result<(ObjId, String), CmdError> {
    if buyer.0 == actor {
        return Err(no_player(typed).into());
    }
    Ok(buyer)
}
/// 17g C3a: `NO_PLAYER` for the name as typed, `player`'s and
/// `other_player`'s one refusal, with its word.
fn no_player(typed: &str) -> Refusal {
    word!(NO_PLAYER, &[("name", typed)])
}

/// `+minds/resell <licence>=<player> price=<credits>`. 17g C3a: parsed,
/// run, then the request line and the seller's tell printed as before.
pub(crate) fn resell(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let args = parse_offer(text)?;
    let outcome = run_resell(ctx, &args)?;
    super::print(ctx, outcome);
    ctx.out.info(
        ctx.actor,
        fill(
            words::RESELL_TELL,
            &[
                ("licence", &args.subject.to_string()),
                ("amount", &amount(args.price_minor)),
            ],
        ),
    );
    Ok(())
}

/// 17g C3a: `+minds/resell` after its parse, for the line and the typed
/// `Resell` alike: the slot (`NOT_OPEN`), the player (`NO_PLAYER`), the
/// actor's own paid licence (`NO_LICENCE`), not the actor itself
/// (`NO_PLAYER`), the action (`NOT_OPEN`), the personal account (`ACCOUNT`),
/// then `RESELLING` and the Offer. The seller's tell is the line command's
/// second line, never part of the `Outcome` (the C3 plan §2.7).
pub(super) fn run_resell(ctx: &mut Ctx<'_>, args: &TradeArguments) -> Result<Outcome, Refusal> {
    super::service(ctx)?;
    let named = player(ctx, &args.player)?;
    let held = super::renew::own_paid(ctx, args.subject)?;
    let (buyer, name) =
        other_player(ctx.actor, named, &args.player).map_err(|_| no_player(&args.player))?;
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
    super::stage(ctx, "resell", &licence, action, parameters, request)
}

/// `+minds/charter <filed>=<player> price=<credits>`. 17g C3a: parsed,
/// run, then the request line and the seller's tell printed as before.
pub(crate) fn charter(ctx: &mut Ctx<'_>, text: &str) -> Result<(), CmdError> {
    let args = parse_offer(text)?;
    let outcome = run_charter(ctx, &args)?;
    super::print(ctx, outcome);
    ctx.out.info(
        ctx.actor,
        fill(
            words::CHARTER_TELL,
            &[
                ("filed", &args.subject.to_string()),
                ("amount", &amount(args.price_minor)),
            ],
        ),
    );
    Ok(())
}

/// 17g C3a: `+minds/charter` after its parse, for the line and the typed
/// `Charter` alike (`run_resell`'s order, with the actor's own filed mind,
/// `NO_FILED`, and `CHARTERING`).
pub(super) fn run_charter(ctx: &mut Ctx<'_>, args: &TradeArguments) -> Result<Outcome, Refusal> {
    super::service(ctx)?;
    let named = player(ctx, &args.player)?;
    let owned = super::author::own_filed(ctx, args.subject)?;
    let (buyer, name) =
        other_player(ctx.actor, named, &args.player).map_err(|_| no_player(&args.player))?;
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
    super::stage(ctx, "charter", &filed, action, parameters, request)
}

/// 17g C2g: `+minds/resell/accept <licence> [price=<credits>]` and
/// `+minds/charter/accept <filed> [price=<credits>]` (module doc): the live
/// offers naming the actor (`resale_offers`, `charter_offers`), then
/// `accept_plan`: its lines and nothing submitted, its refusal, or the one
/// confirmed offer submitted with the request line it names, as the resale's
/// Accept or Fund (the step its view carries) or the pending charter's Fund,
/// paid from the actor's one personal account (`ACCOUNT`). The key's subject
/// is the offer's token.
///
/// 17g C3a: parsed, then `run_accept`. Its offer-listing refusals
/// (`AcceptPlan::Refuse`: `NO_OFFER`, `NO_OFFER_AT` and their charter twins)
/// carry every line in the refusal's text, one per `\n`, and print here as
/// before: one line is the returned error; `NO_OFFER_AT` and the live
/// offers are an error line then info lines (`market_commands::refusal`).
pub(crate) fn accept(ctx: &mut Ctx<'_>, text: &str, named: Named) -> Result<(), CmdError> {
    let args = parse_accept(text)?;
    match run_accept(ctx, &args, named) {
        Ok(outcome) => {
            super::print(ctx, outcome);
            Ok(())
        }
        Err(refused) if offer_listing(refused.word) => {
            let lines = refused.text.split('\n').map(str::to_owned).collect();
            super::say(ctx, super::refusal(lines)?)
        }
        Err(refused) => Err(refused.into()),
    }
}

/// 17g C3a: an accept plan's refusal (`AcceptPlan::Refuse(lines)`) with its
/// word: one line is `NO_OFFER` / `NO_CHARTER_OFFER` (no live offer names
/// the actor); more are `NO_OFFER_AT` / `NO_CHARTER_OFFER_AT` followed by
/// the live offers' lines (`accept_plan`'s only two refusals). The text
/// holds every line, joined with `\n`, for the line command to print whole
/// (`accept`); the typed reply keeps the first (`typed::response`).
pub(super) fn plan_refusal(named: Named, lines: Vec<String>) -> Refusal {
    let word = match (named, lines.len()) {
        (Named::Resale, 1) => "NO_OFFER",
        (Named::Resale, _) => "NO_OFFER_AT",
        (Named::Charter, 1) => "NO_CHARTER_OFFER",
        (Named::Charter, _) => "NO_CHARTER_OFFER_AT",
    };
    Refusal::of(word, lines.join("\n"))
}

/// 17g C3a: whether a refusal's word is one of an accept plan's
/// (`AcceptPlan::Refuse`), whose text holds its lines joined with `\n`.
fn offer_listing(word: &str) -> bool {
    matches!(
        word,
        "NO_OFFER" | "NO_OFFER_AT" | "NO_CHARTER_OFFER" | "NO_CHARTER_OFFER_AT"
    )
}

/// 17g C3a: an accept after its parse, for the line and the typed
/// `ResellAccept` and `CharterAccept` alike: the slot (`NOT_OPEN`), the live
/// offers naming the actor, then the plan: `Outcome::Shown` with the offer
/// lines (no price), the refusal (`NO_OFFER` alone, or `NO_OFFER_AT` with
/// the offer lines after it in the text, one per `\n`), or the confirmed
/// offer staged with `ACCEPTING` / `FUNDING_CHARTER`.
pub(super) fn run_accept(
    ctx: &mut Ctx<'_>,
    args: &AcceptArguments,
    named: Named,
) -> Result<Outcome, Refusal> {
    super::service(ctx)?;
    let (offers, step) = match named {
        Named::Resale => resale_offers(ctx, args.subject)?,
        Named::Charter => (charter_offers(ctx, args.subject)?, None),
    };
    let now = ctx.w.now();
    match accept_plan(named, args.subject, &offers, args.price_minor, now) {
        AcceptPlan::Show(lines) => Ok(Outcome::Shown(lines)),
        AcceptPlan::Refuse(lines) => Err(plan_refusal(named, lines)),
        AcceptPlan::Submit(offer, request) => {
            let definitions = ctx.reg.definitions().ok_or_else(not_open)?;
            let action = match named {
                Named::Resale => MarketCommandActions::capture(definitions)
                    .resale(step.ok_or_else(not_open)?)
                    .ok_or_else(not_open)?,
                Named::Charter => RollCommandActions::capture(definitions)
                    .pending_charter_fund()
                    .ok_or_else(not_open)?,
            };
            let payer = super::personal_account(ctx)?;
            let parameters = match named {
                Named::Resale => resale_accept_parameters(offer.offer, payer),
                Named::Charter => charter_fund_parameters(offer.offer, payer),
            };
            let (verb, subject) = accept_key(named, offer.offer);
            super::stage(ctx, verb, &subject, action, parameters, request)
        }
    }
}

/// 17g C2g: an accept's idempotency verb and subject (`next_key`): the
/// offer's own token, never the licence or filed number the buyer typed.
pub(crate) fn accept_key(named: Named, offer: CommitToken) -> (&'static str, String) {
    let verb = match named {
        Named::Resale => "resell.accept",
        Named::Charter => "charter.accept",
    };
    (verb, offer.to_string())
}

/// 17g C2g: the live resale offers naming the actor on licence `subject`,
/// through the market read's own-only `ResaleOffers`, and the acceptance
/// step their views carry (one licence, one claim census, one step). A
/// licence number nobody offered the actor, a foreign one included, is the
/// empty list; a read that cannot run is `NOT_OPEN`.
fn resale_offers(
    ctx: &Ctx<'_>,
    subject: u64,
) -> Result<(Vec<NamedOffer>, Option<ResaleKind>), Refusal> {
    let Ok(licence) = LicenceId::new(subject) else {
        return Ok((Vec::new(), None));
    };
    let (market, _) = super::market::handles(ctx)?;
    let views = match ctx.read_owner(&market, MarketCommandRequest::resale_offers(licence)) {
        Ok(result) => result.into_resale_offers().ok_or_else(not_open)?,
        Err(OwnerReadError::Refused) => Vec::new(),
        Err(_) => return Err(not_open()),
    };
    let step = views.first().map(|view| view.step());
    let offers = views
        .iter()
        .map(|view| NamedOffer {
            offer: view.offer(),
            price_minor: view.price_minor(),
            expires_at_utc: view.expires_at_utc(),
        })
        .collect();
    Ok((offers, step))
}

/// 17g C2g: the live pending-charter offers naming the actor on the charter
/// of filed mind `subject`, through Roll's own-only `CharterOffers`. A filed
/// number that does not exist (Roll's `Refused`) and a charter nobody offered
/// the actor are both the empty list; a read that cannot run is `NOT_OPEN`.
fn charter_offers(ctx: &Ctx<'_>, subject: u64) -> Result<Vec<NamedOffer>, Refusal> {
    let Ok(filed) = FiledRevisionId::new(subject) else {
        return Ok(Vec::new());
    };
    let service = super::service(ctx)?;
    let request = OriginalReadRequest::command(RollCommandReadRequest::charter_offers(filed));
    let views = match ctx.read_owner(&service.roll, request) {
        Ok(result) => result
            .into_command()
            .and_then(RollCommandReadResult::into_charter_offers)
            .ok_or_else(not_open)?,
        Err(OwnerReadError::Refused) => Vec::new(),
        Err(_) => return Err(not_open()),
    };
    Ok(views
        .iter()
        .map(|view| NamedOffer {
            offer: view.offer(),
            price_minor: view.price_minor(),
            expires_at_utc: view.expires_at_utc(),
        })
        .collect())
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

/// The paid-resale Accept's and Fund's wire (`AcceptParameters`).
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
/// payer as a decimal string.
pub(crate) fn charter_fund_parameters(
    offer: CommitToken,
    payer: mush_economy::AccountId,
) -> serde_json::Value {
    json!({"offer": offer, "payer": payer.0.to_string()})
}

// ---- the named buyer's listing and price rule ----

/// One offer naming the actor, as the offer indexes answer it: the offer's
/// committed token, frozen price and expiry (world UTC seconds).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NamedOffer {
    pub(crate) offer: CommitToken,
    pub(crate) price_minor: i64,
    pub(crate) expires_at_utc: i64,
}

/// A resale offer (on a licence) or a charter offer (on a filed mind).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Named {
    Resale,
    Charter,
}

/// What an accept does with the live offers naming the actor.
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
pub(crate) fn minutes_left(until: i64, now: i64) -> i64 {
    until.saturating_sub(now).max(0).saturating_add(59) / 60
}

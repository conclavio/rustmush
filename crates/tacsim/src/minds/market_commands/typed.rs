//! 17g C3a (the C3 plan §2.1, §2.3): the typed entrance to 17g C2's player
//! commands. A `MindRequest::Market { action }` reaches `submit` from the
//! `mind` invoke handler (`workshop::structured`), dispatched beside `Trace`
//! after the connection-binding check and BEFORE the workshop's principal
//! gate (the C3a brief §4), so a market request never spends an author
//! session, and C2's own rule (`market_commands::stage`: a live, connected,
//! non-GOING player with an account, else `NOT_OPEN`) is the only principal
//! check it meets.
//!
//! For the eleven `+minds` forms that is exactly the line's rule: the line
//! runs no workshop gate either. `@mind/publish` and `@mind/withdraw` are
//! `@mind` switches, and the line's `WorkshopCommands::dispatch` runs the
//! workshop's principal gate (`WorkshopCommands::principal`: an account,
//! then an author session from the executor) before every `@mind/<switch>`
//! that reaches C2. So for those two the surfaces differ, a recorded
//! difference: where that gate refuses (an actor without an account, a
//! stopped executor, the author sessions at capacity) the line prints the
//! workshop's sentence while the typed `Publish` and `Withdraw` run C2's
//! checks and answer C2's words (an actor without an account reads the
//! refusal its selection meets first, as the
//! `typed_refusals_are_the_line_refusals_byte_for_byte` server test pins).
//! Everywhere else they refuse alike.
//!
//! Each `MarketAction` is converted into the very selection the line
//! command's parser builds from the equivalent line (`select`; the
//! `typed_actions_build_the_line_selections` lib test pins the two equal),
//! and run through the same `run_*` function the line command runs (`run`).
//! So the provider sees the same parameters, the key has the line's shape
//! (one more attempt on the slot's per-second ordinal: a typed and a line
//! request are two requests, as two retyped lines are), and every refusal is
//! a C2 word, byte for byte:
//! - a refusal is `MindResponse::Error { code: the word's constant name in
//!   lower case, message: its text }` (`response`), the message cut at its
//!   first line break (an accept's `NO_OFFER_AT` carries the offer lines
//!   after it for the line command; the typed reply is the refusal alone,
//!   and the client asks again without a price to see the offers, the C3
//!   plan §2.7);
//! - a staged submission is `Staged { line }`, the request line the line
//!   prints (the seller's tell after `RESELLING` / `CHARTERING` is the line
//!   command's second line and is not carried); the three renewal steps,
//!   which print nothing on the line, answer `Staged { RENEW_HANDED }`;
//! - an accept without a price is `Shown { lines }`, the offer lines.
//!
//! Nothing here names an account, a token, a digest or a recipient's
//! account: the server builds every parameter from the actor's own state,
//! through the same reads the line command makes. A provider's refusal after
//! staging is the driver's one line on the actor's ordinary output, as for
//! the line.

use mush_core::Ctx;
use mush_proto::{MarketAction, MarketListingForm, MindResponse};

use super::market::{BuySelection, PublishArguments, Settle};
use super::renew::{Renew, RenewSelection};
use super::trade::{AcceptArguments, Named, TradeArguments, TradeRefusal};
use super::words::{Outcome, Refusal, word};

/// 17g C3a: what one typed action selects, the line parser's selection for
/// the equivalent line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Selection {
    /// `+minds/buy <code>`.
    Quote(BuySelection),
    /// `+minds/buy/pay <code>`.
    Pay(BuySelection),
    /// `+minds/buy/complete <code>`.
    Complete(BuySelection),
    /// `+minds/buy/refund <code>`.
    Refund(BuySelection),
    /// `+minds/renew <licence>` and its `/pay` and `/decline`.
    Renew(RenewSelection),
    /// `+minds/resell <licence>=<player> price=<credits>`.
    Resell(TradeArguments),
    /// `+minds/resell/accept <licence> [price=<credits>]`.
    ResellAccept(AcceptArguments),
    /// `+minds/charter <filed>=<player> price=<credits>`.
    Charter(TradeArguments),
    /// `+minds/charter/accept <filed> [price=<credits>]`.
    CharterAccept(AcceptArguments),
    /// `@mind/publish <filed> form=... [royalty=...] [transfer=...]`.
    Publish(PublishArguments),
    /// `@mind/withdraw <filed>`.
    Withdraw(u64),
}

/// 17g C3a: one typed market action, answered: its selection, its run, and
/// the reply (`response`). The reply is what the actor's own connection
/// receives as the invoke result; the outcome of a staged submission follows
/// on the actor's ordinary output.
pub(crate) fn submit(ctx: &mut Ctx<'_>, action: &MarketAction) -> MindResponse {
    response(select(action).and_then(|selection| run(ctx, selection)))
}

/// 17g C3a: the selection the line parser builds for the equivalent line.
/// The numbers pass as typed (a zero reaches the own-record read and refuses
/// as a missing number does); the player name is trimmed, as the line trims
/// it, and resolved by C2's resolver at run; a price goes through C2's
/// credit grammar (`BAD_PRICE`); a listing's form, royalty and transfer mark
/// follow `publish_arguments`' rules (`None` royalty is 0, a sealed listing
/// takes 0, `None` transfer is transferable).
pub(super) fn select(action: &MarketAction) -> Result<Selection, Refusal> {
    let buy = |code: &mush_proto::ListingCode| BuySelection {
        code: code.as_str().to_owned(),
    };
    let renew = |licence: u64, which: Renew| Selection::Renew(RenewSelection { licence, which });
    let offer = |subject: u64, player: &str, price: &str| -> Result<TradeArguments, Refusal> {
        Ok(TradeArguments {
            subject,
            player: player.trim().to_owned(),
            price_minor: super::trade::price(price).map_err(TradeRefusal::refusal)?,
        })
    };
    let accept = |subject: u64, price: Option<&String>| -> Result<AcceptArguments, Refusal> {
        Ok(AcceptArguments {
            subject,
            price_minor: price
                .map(|text| super::trade::price(text).map_err(TradeRefusal::refusal))
                .transpose()?,
        })
    };
    Ok(match action {
        MarketAction::Quote { code } => Selection::Quote(buy(code)),
        MarketAction::Pay { code } => Selection::Pay(buy(code)),
        MarketAction::Complete { code } => Selection::Complete(buy(code)),
        MarketAction::Refund { code } => Selection::Refund(buy(code)),
        MarketAction::Renew { licence } => renew(*licence, Renew::Status),
        MarketAction::RenewPay { licence } => renew(*licence, Renew::Pay),
        MarketAction::RenewDecline { licence } => renew(*licence, Renew::Decline),
        MarketAction::Resell {
            licence,
            player,
            price,
        } => Selection::Resell(offer(*licence, player, price)?),
        MarketAction::ResellAccept { licence, price } => {
            Selection::ResellAccept(accept(*licence, price.as_ref())?)
        }
        MarketAction::Charter {
            filed,
            player,
            price,
        } => Selection::Charter(offer(*filed, player, price)?),
        MarketAction::CharterAccept { filed, price } => {
            Selection::CharterAccept(accept(*filed, price.as_ref())?)
        }
        MarketAction::Publish {
            filed,
            form,
            royalty,
            transfer,
        } => Selection::Publish(publish(*filed, *form, *royalty, *transfer)?),
        MarketAction::Withdraw { filed } => Selection::Withdraw(*filed),
    })
}

/// `publish_arguments`' result for the typed fields: the List offer's form
/// choice and the word the reply prints, the royalty (`BAD_ROYALTY` above
/// 300, or non-zero on a sealed listing) and the transfer mark; no `price=`,
/// `seats=` or `term=` (the market's own terms).
fn publish(
    filed: u64,
    form: MarketListingForm,
    royalty: Option<u16>,
    transfer: Option<bool>,
) -> Result<PublishArguments, Refusal> {
    let (form, shown) = match form {
        MarketListingForm::Sealed => ("sealed", "sealed"),
        MarketListingForm::Open => ("open", "open"),
        MarketListingForm::OpenResale => ("open_resale", "open-resale"),
    };
    let royalty = royalty.unwrap_or(0);
    if royalty > 300 || (form == "sealed" && royalty != 0) {
        return Err(word!(BAD_ROYALTY));
    }
    Ok(PublishArguments {
        filed,
        form,
        shown,
        royalty,
        transferable: transfer.unwrap_or(true),
        price: None,
        seats: None,
        term: None,
    })
}

/// 17g C3a: the line command's run for the selection.
pub(super) fn run(ctx: &mut Ctx<'_>, selection: Selection) -> Result<Outcome, Refusal> {
    match selection {
        Selection::Quote(buy) => super::market::run_quote(ctx, &buy),
        Selection::Pay(buy) => super::market::run_pay(ctx, &buy),
        Selection::Complete(buy) => super::market::run_settle(ctx, &buy, Settle::Complete),
        Selection::Refund(buy) => super::market::run_settle(ctx, &buy, Settle::Refund),
        Selection::Renew(renew) => super::renew::run_renew(ctx, renew),
        Selection::Resell(args) => super::trade::run_resell(ctx, &args),
        Selection::ResellAccept(args) => super::trade::run_accept(ctx, &args, Named::Resale),
        Selection::Charter(args) => super::trade::run_charter(ctx, &args),
        Selection::CharterAccept(args) => super::trade::run_accept(ctx, &args, Named::Charter),
        Selection::Publish(args) => super::market::run_publish(ctx, &args),
        Selection::Withdraw(filed) => super::author::run_withdraw(ctx, filed),
    }
}

/// 17g C3a: the typed reply: `Staged { line }`, `Shown { lines }`, or
/// `Error { code, message }` with the word's constant name in lower case and
/// the refusal's first line (`Refusal`'s doc).
pub(super) fn response(result: Result<Outcome, Refusal>) -> MindResponse {
    match result {
        Ok(Outcome::Staged(line)) => MindResponse::Staged { line },
        Ok(Outcome::Shown(lines)) => MindResponse::Shown { lines },
        Err(refusal) => MindResponse::Error {
            code: refusal.word.to_ascii_lowercase(),
            message: refusal
                .text
                .split('\n')
                .next()
                .unwrap_or_default()
                .to_owned(),
        },
    }
}

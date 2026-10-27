//! 17g C2a (C2 plan §2.2, the C2a brief's words): every sentence the author
//! commands print, verbatim. `{reference}` is the resolved canonical
//! `name@rev`, `{arguments}` the command's argument text, `{filed}` and
//! `{licence}` the typed decimal number, `{charter}` and `{name}` the names
//! the command uses. A refusal names only what the actor typed or owns, so a
//! foreign number reads exactly like one that does not exist.
//!
//! The usage lines are the `@mind` `HELP` lines (`workshop::commands`).

pub(crate) const NOT_OPEN: &str = "The mind market is not open here.";
pub(crate) const BUSY: &str =
    "Your last market request is still being recorded; try again in a moment.";
pub(crate) const ACCOUNT: &str = "This needs exactly one personal credit account in your name.";
pub(crate) const NO_REVISION: &str = "You have no saved revision {reference}.";
pub(crate) const NOT_RATED: &str =
    "{reference} has no finished Registry rating. Rate it with @mind/rate {reference}.";
pub(crate) const RATING_BUSY: &str =
    "The Registry is already working on one of your revisions; wait until it says it is finished.";
pub(crate) const RESERVING: &str = "Reserving a charter for this filing first. Once that is confirmed, type @mind/file {arguments} again.";
pub(crate) const FILING: &str = "Filing {reference} as {charter}.";
pub(crate) const NO_FILED: &str = "You own no filed mind {filed}.";
pub(crate) const UPDATING: &str = "Filing {reference} on the line of filed mind {filed}.";
pub(crate) const WITHDRAWING: &str = "Withdrawing filed revision {filed}.";
pub(crate) const ATTESTING: &str = "Renewing your owner's licence for filed mind {filed}.";
pub(crate) const FORKING: &str = "Forking licence {licence} as {name}.";

/// The Registry service's own refusal, which `@mind/rate` shares with
/// `@registry/rate` (`registry_service::command`).
pub(crate) const RATING_UNAVAILABLE: &str = "The owned Registry computation is unavailable.";

// 17g C2c (the C2c brief's words, verbatim): the listings, a listing's
// terms, the quote step, publication and the owner half of the holdings.
// `{code}` is a listing's 8-character code; `{name}` a charter's public Roll
// name, cut to 24 characters in a listing and 20 in the holdings; a
// `{key:<n}` or `{key:>n}` fill is padded to n characters (`fill`).
pub(crate) const COUNTER: &str = "Listing and buying minds happen at a market counter.";
pub(crate) const EMPTY: &str = "No minds are for sale at this counter.";
pub(crate) const HEAD: &str = "Minds for sale here (term and Grace in IC days):";
pub(crate) const ROW: &str =
    "  {code}  {name:<24}  {form:<11}  class {class:<3}  {price:>12}  {term}+{grace} days  {flags}";
pub(crate) const FOOT: &str =
    "Use +minds <code> for the full terms and +minds/buy <code> to ask for a quote.";
pub(crate) const MORE: &str = "More: +minds/next {code}";
pub(crate) const NO_LISTING: &str = "No listing here has the code {code}.";
pub(crate) const AMBIGUOUS: &str =
    "Two listings here share the code {code}; use +actions and +act for these.";
pub(crate) const DETAIL: &str = "Listing {code}: {name}, {form}{derived}, class {class} and above.";
pub(crate) const TERMS_LINE: &str =
    "One seat for {term} IC days plus {grace} days of Grace; the licence {resale}.";
pub(crate) const SPLIT: &str =
    "Price {price}: Registry {levy}%{venue}{parents}, charter owner the rest.";
pub(crate) const QUOTING: &str =
    "Asking for a quote on listing {code}. +minds {code} shows it once it is confirmed.";
/// 17g C2e (C2e plan §2.1 item 4c): `{flags}` is empty for a transferable
/// listing of an original line, else `; ` and `ROW`'s flag words
/// (`market::publishing_line`).
pub(crate) const PUBLISHING: &str = "Listing filed mind {filed} here as {form}{flags}.";
pub(crate) const TERMS: &str = "Listings here sell at {price} for one seat and {term} IC days; per-listing terms are not available yet.";
pub(crate) const BAD_FORM: &str = "form is sealed, open or open-resale.";
pub(crate) const BAD_ROYALTY: &str =
    "royalty is a whole number from 0 to 300 per mille, and 0 on a sealed listing.";
pub(crate) const BAD_TRANSFER: &str = "transfer is yes or no.";
pub(crate) const HOLD_HEAD: &str = "Your mind licences:";
pub(crate) const HOLD_OWNER: &str = "  filed {filed:<8}  {kind:<9}  {name:<20}  {phase}{flags}";
pub(crate) const HOLD_MORE: &str = "More: +minds/holdings {section} {cursor}";

// 17g C2c's words for the steps that read the actor's own quote, funded
// purchase and paid licences, live since 17g C2g over 17g C2f's owner
// indexes (they waited in `words::pending` until then; the names are
// unchanged). `HOLD_EMPTY` is the bare `+minds/holdings`' answer for an
// actor with nothing in either section, never a paged section's.
pub(crate) const HOLD_EMPTY: &str = "You hold no mind licences.";
pub(crate) const YOUR_QUOTE: &str =
    "Your quote: {price}, good for {seconds} more seconds. Pay it with +minds/buy/pay {code}.";
pub(crate) const QUOTE_EXPIRED: &str =
    "Your quote has expired. Ask for a new one with +minds/buy {code}.";
pub(crate) const YOUR_ESCROW: &str =
    "Your payment is held in escrow. Take the licence with +minds/buy/complete {code}.";
pub(crate) const ASK: &str = "Ask for a quote with +minds/buy {code}.";
pub(crate) const NO_QUOTE: &str =
    "You have no quote on listing {code}. Ask for one with +minds/buy {code}.";
pub(crate) const PAYING: &str = "Paying your quote on listing {code} into escrow.";
pub(crate) const NO_PURCHASE: &str = "You have no paid purchase waiting on listing {code}.";
pub(crate) const COMPLETING: &str = "Taking the licence from listing {code}.";
pub(crate) const REFUNDING: &str = "Asking for your payment on listing {code} back.";
pub(crate) const HOLD_PAID: &str =
    "  licence {licence:<8}  {kind:<9}  {name:<20}  {form:<11}  {phase}{flags}";
pub(crate) const HOLD_WAIT_HEAD: &str = "Purchases waiting:";
pub(crate) const HOLD_WAIT: &str =
    "  listing {code}  paid into escrow: +minds/buy/complete {code} or +minds/buy/refund {code}";

// 17g C2d (the C2d brief's words, verbatim): renewals, title trades and the
// Registry operators. `{player}` is the resolved player's name; `{price}`
// the credit rendering (`launch::credits`); `{amount}` the same value as
// `price=` spells it (`trade::amount`); `{days}`, `{hours}` and `{term}`/
// `{grace}` whole IC days or hours, rounded up.
pub(crate) const NO_LICENCE: &str = "You hold no licence {licence}.";
pub(crate) const RENEW_NOT_DUE: &str =
    "Licence {licence} is not due for renewal; its term has {days} IC days left.";
pub(crate) const RENEWING: &str = "Asking for a renewal offer on licence {licence}. +minds/renew {licence} shows it once it is confirmed.";
pub(crate) const RENEW_OFFER: &str = "Licence {licence} renews for {price} ({term}+{grace} IC days); the offer is open {hours} more IC hours.";
pub(crate) const RENEW_HOW: &str =
    "Pay it with +minds/renew/pay {licence}, or turn it down with +minds/renew/decline {licence}.";
pub(crate) const RENEW_FUNDED: &str =
    "Your renewal payment for licence {licence} is held; the renewal completes on its own.";
pub(crate) const NO_RENEWAL: &str = "Licence {licence} has no renewal offer waiting for you. Ask for one with +minds/renew {licence}.";
pub(crate) const RENEW_PAYING: &str =
    "Paying {price} for the renewal of licence {licence} into escrow; it completes on its own.";
pub(crate) const RENEW_DECLINING: &str = "Turning down the renewal offer on licence {licence}.";
pub(crate) const RESELLING: &str = "Offering licence {licence} to {player} for {price}.";
pub(crate) const RESELL_TELL: &str =
    "Once that is confirmed, they accept with +minds/resell/accept {licence} price={amount}.";
pub(crate) const CHARTERING: &str =
    "Offering filed mind {filed} and its charter to {player} for {price}.";
pub(crate) const CHARTER_TELL: &str =
    "Once that is confirmed, they accept with +minds/charter/accept {filed} price={amount}.";
pub(crate) const NO_PLAYER: &str = "No player is named {name}.";
pub(crate) const BAD_PRICE: &str =
    "price is in credits, from 0.01 to 1000000, such as price=12.50.";
pub(crate) const RECALLING: &str = "Recalling filed revision {filed}.";
pub(crate) const REVOKING: &str = "Revoking licence {licence}.";
pub(crate) const REGISTRY_OFF: &str = "Registry recall and revocation are not configured here.";

/// 17g C2d: the brief's words for the named buyer's accepts, which list and
/// select the live offers naming the actor (`trade::offer_lines`,
/// `trade::accept_plan`). Live since 17g C2g over 17g C2f's per-buyer offer
/// indexes (the market's `ResaleOffers` and Roll's `CharterOffers` reads).
pub(crate) mod named {
    pub(crate) const OFFER: &str = "Licence {licence} is offered to you for {price}; the offer is open {minutes} more minutes.";
    pub(crate) const ACCEPT_HOW: &str =
        "Accept it with +minds/resell/accept {licence} price={amount}.";
    pub(crate) const NO_OFFER: &str = "You have no offer for licence {licence}.";
    pub(crate) const NO_OFFER_AT: &str = "None of your offers for licence {licence} is at {price}.";
    pub(crate) const CHARTER_OFFER: &str = "Filed mind {filed} and its charter are offered to you for {price}; the offer is open {minutes} more minutes.";
    pub(crate) const CHARTER_ACCEPT_HOW: &str =
        "Accept it with +minds/charter/accept {filed} price={amount}.";
    pub(crate) const NO_CHARTER_OFFER: &str = "You have no offer for filed mind {filed}.";
    pub(crate) const NO_CHARTER_OFFER_AT: &str =
        "None of your offers for filed mind {filed} is at {price}.";
    pub(crate) const ACCEPTING: &str = "Accepting the offer for licence {licence} at {price}.";
    pub(crate) const FUNDING_CHARTER: &str = "Paying {price} for filed mind {filed} into escrow; the transfer settles once no sortie under it is out.";
}

/// `template` with each `{key}` replaced by its value, in one pass over the
/// template, so text the player typed is never itself substituted. 17g C2c:
/// `{key:<n}` pads the value with spaces on the right to n characters and
/// `{key:>n}` on the left; a longer value is never cut here (the caller cuts
/// names first).
pub(crate) fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let (before, tail) = rest.split_at(open);
        out.push_str(before);
        let value = tail.find('}').and_then(|close| {
            let field = tail.get(1..close)?;
            let (key, pad) = match field.split_once(':') {
                Some((key, spec)) => (key, Some(spec)),
                None => (field, None),
            };
            let (_, value) = values.iter().find(|(name, _)| *name == key)?;
            let value = match pad {
                None => (*value).to_owned(),
                Some(spec) => padded(value, spec)?,
            };
            Some((value, close))
        });
        match value {
            Some((value, close)) => {
                out.push_str(&value);
                rest = tail.get(close + 1..).unwrap_or_default();
            }
            None => {
                out.push('{');
                rest = tail.get(1..).unwrap_or_default();
            }
        }
    }
    out.push_str(rest);
    out
}

/// `value` padded to the width `spec` names: `<n` on the right, `>n` on the
/// left, in characters. Any other spec is not a fill.
fn padded(value: &str, spec: &str) -> Option<String> {
    let (left, width) = match spec.split_at_checked(1)? {
        ("<", width) => (false, width),
        (">", width) => (true, width),
        _ => return None,
    };
    let width: usize = width.parse().ok()?;
    let gap = " ".repeat(width.saturating_sub(value.chars().count()));
    Some(if left {
        format!("{gap}{value}")
    } else {
        format!("{value}{gap}")
    })
}

/// `name` cut to `limit` characters.
pub(crate) fn cut(name: &str, limit: usize) -> String {
    name.chars().take(limit).collect()
}

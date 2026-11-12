//! **The mind market's sentences, as the bot reads them** (doc 44 MW.5; doc 17g C4 item 3,
//! "`cargo test -p mush-bot` green on every new sentence").
//!
//! One `pub const` needle per player-facing sentence of six game modules, grouped in one
//! module per family and named exactly as the game's constant is named. A needle is a literal
//! fragment of the game's sentence: it never crosses a `{placeholder}`, so it is found in the
//! filled line whatever the fill, and it is chosen to tell its sentence from the rest of its
//! family (the longest placeholder-free fragment, unless that fragment is shared).
//!
//! This is the table only. No lane reads these yet (doc 44 MW.19 owns the bot's market lane),
//! and nothing here is a game-crate dependency: the needles are the bot's own text. The tests
//! below hold the table to the game through the crate's development dependencies (`tacsim`,
//! `mush-market`), in the role [`super::lines`]' tests play for the mining loop:
//!
//! * **in step**: every needle is a literal fragment of the game constant it names, so a
//!   reworded sentence fails `cargo test -p mush-bot`;
//! * **complete**: a census reads each game module's source (`include_str!`) and fails naming
//!   any `&str` constant declared there with no row here and no listed exclusion, so a sentence
//!   added to one of these modules fails the bot's tests until the bot learns it;
//! * **distinct**: within a family no needle is found in another member's sentence, except
//!   the pairs listed in the test module's overlap tables, each with its reason.
//!
//! Two sentences are the same text in two families: `dispatch::MIND_NO_LICENCE` and
//! `market::NO_LICENCE` ("You hold no licence {licence}."), and `dispatch::MIND_NOT_FOR_ORG`,
//! which the game re-exports from `market::NOT_FOR_ORG`. A reader that must tell them apart
//! does it by the command it typed, not by the line.
//!
//! Three needles keep a sentence's inner spacing, because the sentence is a table row whose
//! only words are shared with the family's prose: `market::ROW`, `market::HOLD_OWNER` and
//! `market::HOLD_PAID` match the row's two-space gaps, so a reader must not collapse runs of
//! spaces before it looks for them.

/// `+enduser`, the Registry's Class III end-user registration (doc 44 MW.23):
/// `tacsim::launch::end_user`.
pub mod end_user {
    /// `tacsim::launch::end_user::END_USER_HELP`: a phrase of the help's first sentence.
    pub const END_USER_HELP: &str =
        "without it no shop builds you a Class III core or hands one over";
    /// `tacsim::launch::end_user::END_USER_NOT_OPEN`.
    pub const END_USER_NOT_OPEN: &str =
        "The Registry takes no end-user registrations on this world.";
    /// `tacsim::launch::end_user::END_USER_UNLISTED`.
    pub const END_USER_UNLISTED: &str =
        "The Registry does not list you as a Class III end user. +enduser/apply registers you:";
    /// `tacsim::launch::end_user::END_USER_LISTED`.
    pub const END_USER_LISTED: &str = "The Registry lists you as a Class III end user, account #";
    /// `tacsim::launch::end_user::END_USER_SUSPENDED`.
    pub const END_USER_SUSPENDED: &str =
        "The Registry has suspended your Class III end-user registration, account #";
    /// `tacsim::launch::end_user::END_USER_NO_CREDIT`.
    pub const END_USER_NO_CREDIT: &str = "You hold no credit account to pay the Registry from.";
    /// `tacsim::launch::end_user::END_USER_FUNDS`.
    pub const END_USER_FUNDS: &str = "An end-user registration costs";
    /// `tacsim::launch::end_user::END_USER_PREVIEW`.
    pub const END_USER_PREVIEW: &str = "The Registry registers you as a Class III end user for";
    /// `tacsim::launch::end_user::END_USER_DONE`.
    pub const END_USER_DONE: &str = "The Registry lists you as a Class III end user:";
    /// `tacsim::launch::end_user::END_USER_FAILED`.
    pub const END_USER_FAILED: &str = "The Registry could not register you; nothing was paid.";
    /// `tacsim::launch::end_user::END_USER_USAGE`.
    pub const END_USER_USAGE: &str = "Usage: +enduser[/show]  or  +enduser/apply [go]";
}

/// The renewal service's player lines (`+minds/renew`, `@mindrenewal`; doc 44 MW.2's
/// `RENEW_STOPPED`): `mush_market::mind_renewal::player`.
pub mod renewal {
    /// `mush_market::mind_renewal::player::QUEUED_LINE`.
    pub const QUEUED_LINE: &str =
        "Renewal request queued. Status or its acknowledged receipt establishes completion.";
    /// `mush_market::mind_renewal::player::UNRESOLVED_LINE`.
    pub const UNRESOLVED_LINE: &str =
        "A renewal request is still unresolved; inspect status before retrying.";
    /// `mush_market::mind_renewal::player::RENEW_NOT_DUE`.
    pub const RENEW_NOT_DUE: &str = "is not due for renewal; its term has";
    /// `mush_market::mind_renewal::player::RENEWING`.
    pub const RENEWING: &str = "Asking for a renewal offer on licence";
    /// `mush_market::mind_renewal::player::RENEW_OFFER`.
    pub const RENEW_OFFER: &str = "IC days); the offer is open";
    /// `mush_market::mind_renewal::player::RENEW_HOW`.
    pub const RENEW_HOW: &str = "or turn it down with +minds/renew/decline";
    /// `mush_market::mind_renewal::player::RENEW_FUNDED`.
    pub const RENEW_FUNDED: &str = "is held; the renewal completes on its own.";
    /// `mush_market::mind_renewal::player::RENEW_PAYING`.
    pub const RENEW_PAYING: &str = "into escrow; it completes on its own.";
    /// `mush_market::mind_renewal::player::RENEW_DECLINING`.
    pub const RENEW_DECLINING: &str = "Turning down the renewal offer on licence";
    /// `mush_market::mind_renewal::player::RENEW_STOPPED`.
    pub const RENEW_STOPPED: &str = "Nothing further will be tried.";
}

/// A launch's mind claim (doc 23 S.4b): the claim sentences of
/// `tacsim::surface::excursion`.
pub mod claim {
    /// `tacsim::surface::excursion::CLAIM_STILL_OPEN`.
    pub const CLAIM_STILL_OPEN: &str = "that rig's mind claim is still open";
    /// `tacsim::surface::excursion::CLAIM_REFUSED`.
    pub const CLAIM_REFUSED: &str = "The Registry did not admit the mind's claim; the launch is cancelled and the rig stays in its bay.";
    /// `tacsim::surface::excursion::CLAIM_CANCELLED`.
    pub const CLAIM_CANCELLED: &str =
        "The launch could not start; the mind's claim is closed and the rig stays in its bay.";
    /// `tacsim::surface::excursion::CLAIM_NOT_OPEN`.
    pub const CLAIM_NOT_OPEN: &str = "that rig's mind claim has not opened yet";
}

/// A launch's `mind=` refusals (doc 23 S.4b, doc 17h M8e): `tacsim::surface::dispatch`.
pub mod dispatch {
    /// `tacsim::surface::dispatch::MIND_PENDING`.
    pub const MIND_PENDING: &str =
        "is changing hands; it is not sealed aboard until the transfer settles.";
    /// `tacsim::surface::dispatch::MIND_NO_LICENCE`.
    pub const MIND_NO_LICENCE: &str = "You hold no licence";
    /// `tacsim::surface::dispatch::MIND_GOVERNED_DRAFT`.
    pub const MIND_GOVERNED_DRAFT: &str =
        "A governed core seals no unfiled draft; name a filed licence.";
    /// `tacsim::surface::dispatch::MIND_NO_DRAFT`.
    pub const MIND_NO_DRAFT: &str = "You have no draft";
    /// `tacsim::surface::dispatch::MIND_NO_CORE`.
    pub const MIND_NO_CORE: &str = "That rig carries no core; only Standard Doctrine flies it.";
    /// `tacsim::surface::dispatch::MIND_SEATS_HELD`.
    pub const MIND_SEATS_HELD: &str = "seats; bring a hull home first.";
    /// `tacsim::surface::dispatch::MIND_UNAVAILABLE`.
    pub const MIND_UNAVAILABLE: &str = "Minds cannot be sealed aboard here.";
    /// `tacsim::surface::dispatch::MIND_UNTITLED`.
    pub use crate::mine::lines::MIND_UNTITLED;
    /// `tacsim::surface::dispatch::MIND_NOT_FOR_ORG`: the game re-exports `tacsim::minds::words::NOT_FOR_ORG`.
    pub const MIND_NOT_FOR_ORG: &str = "You do not act for that organisation.";
}

/// A field unit's mind taking it home while its pilot is away (doc 44 MW.7):
/// `tacsim::field::absent`.
pub mod absent {
    /// `tacsim::field::absent::MIND_HAS_IT`.
    pub const MIND_HAS_IT: &str = "its mind has the controls and is taking it home. It fires only on a unit that fires on it.";
    /// `tacsim::field::absent::TAKEN_HOME`.
    pub const TAKEN_HOME: &str = "passed to its mind, which makes for its carrier's ramp and fires only on a unit that fires on it.";
    /// `tacsim::field::absent::BACK_IN_SEAT`.
    pub const BACK_IN_SEAT: &str =
        "its mind stands down and what it had pending is dropped. +focus #";
    /// `tacsim::field::absent::WENT_HOME`.
    pub const WENT_HOME: &str = "out of the field at its carrier's ramp.";
    /// `tacsim::field::absent::LEFT_AT_RAMP`.
    pub const LEFT_AT_RAMP: &str = "out of the field, but it could not be put aboard: it stands beside your carrier's ramp. Recover it yourself.";
    /// `tacsim::field::absent::NOT_FIT`.
    pub const NOT_FIT: &str = "its mind keeps the controls.";
    /// `tacsim::field::absent::GONE_HOME`.
    pub const GONE_HOME: &str = "breaks off for its carrier's ramp under its mind:";
}

/// The `+minds`, `@mind/*` and `@registry/*` command family (17g C2, doc 17h M8):
/// `tacsim::minds::words`.
pub mod market {
    /// `tacsim::minds::words::NOT_OPEN`.
    pub const NOT_OPEN: &str = "The mind market is not open here.";
    /// `tacsim::minds::words::BUSY`.
    pub const BUSY: &str =
        "Your last market request is still being recorded; try again in a moment.";
    /// `tacsim::minds::words::ACCOUNT`.
    pub const ACCOUNT: &str = "This needs exactly one personal credit account in your name.";
    /// `tacsim::minds::words::NO_REVISION`.
    pub const NO_REVISION: &str = "You have no saved revision";
    /// `tacsim::minds::words::NOT_RATED`.
    pub const NOT_RATED: &str = "has no finished Registry rating. Rate it with @mind/rate";
    /// `tacsim::minds::words::RATING_BUSY`.
    pub const RATING_BUSY: &str = "The Registry is already working on one of your revisions; wait until it says it is finished.";
    /// `tacsim::minds::words::RESERVING`.
    pub const RESERVING: &str =
        "Reserving a charter for this filing first. Once that is confirmed, type @mind/file";
    /// `tacsim::minds::words::FILING`: shared with `UPDATING` (an allowed overlap).
    pub const FILING: &str = "Filing ";
    /// `tacsim::minds::words::NO_FILED`.
    pub const NO_FILED: &str = "You own no filed mind";
    /// `tacsim::minds::words::UPDATING`.
    pub const UPDATING: &str = "on the line of filed mind";
    /// `tacsim::minds::words::WITHDRAWING`.
    pub const WITHDRAWING: &str = "Withdrawing filed revision";
    /// `tacsim::minds::words::ATTESTING`.
    pub const ATTESTING: &str = "Renewing your owner's licence for filed mind";
    /// `tacsim::minds::words::FORKING`.
    pub const FORKING: &str = "Forking licence";
    /// `tacsim::minds::words::RATING_UNAVAILABLE`.
    pub const RATING_UNAVAILABLE: &str = "The owned Registry computation is unavailable.";
    /// `tacsim::minds::words::COUNTER`.
    pub const COUNTER: &str = "Listing and buying minds happen at a market counter.";
    /// `tacsim::minds::words::EMPTY`.
    pub const EMPTY: &str = "No minds are for sale at this counter.";
    /// `tacsim::minds::words::HEAD`.
    pub const HEAD: &str = "Minds for sale here (term and Grace in IC days):";
    /// `tacsim::minds::words::ROW`: the row's two-space gap before `class`, which `DETAIL` does not have.
    pub const ROW: &str = "  class ";
    /// `tacsim::minds::words::FOOT`.
    pub const FOOT: &str =
        "Use +minds <code> for the full terms and +minds/buy <code> to ask for a quote.";
    /// `tacsim::minds::words::MORE`.
    pub const MORE: &str = "More: +minds/next";
    /// `tacsim::minds::words::NO_LISTING`.
    pub const NO_LISTING: &str = "No listing here has the code";
    /// `tacsim::minds::words::AMBIGUOUS`.
    pub const AMBIGUOUS: &str = "use +actions and +act for these.";
    /// `tacsim::minds::words::DETAIL`.
    pub const DETAIL: &str = "and above.";
    /// `tacsim::minds::words::TERMS_LINE`.
    pub const TERMS_LINE: &str = "days of Grace; the licence";
    /// `tacsim::minds::words::SPLIT`.
    pub const SPLIT: &str = "charter owner the rest.";
    /// `tacsim::minds::words::QUOTING`.
    pub const QUOTING: &str = "shows it once it is confirmed.";
    /// `tacsim::minds::words::PUBLISHING`.
    pub const PUBLISHING: &str = "Listing filed mind";
    /// `tacsim::minds::words::TERMS`.
    pub const TERMS: &str = "IC days; per-listing terms are not available yet.";
    /// `tacsim::minds::words::BAD_FORM`.
    pub const BAD_FORM: &str = "form is sealed, open or open-resale.";
    /// `tacsim::minds::words::BAD_ROYALTY`.
    pub const BAD_ROYALTY: &str =
        "royalty is a whole number from 0 to 300 per mille, and 0 on a sealed listing.";
    /// `tacsim::minds::words::BAD_TRANSFER`.
    pub const BAD_TRANSFER: &str = "transfer is yes or no.";
    /// `tacsim::minds::words::HOLD_HEAD`.
    pub const HOLD_HEAD: &str = "Your mind licences:";
    /// `tacsim::minds::words::HOLD_OWNER`: the owner row's two-space indent.
    pub const HOLD_OWNER: &str = "  filed ";
    /// `tacsim::minds::words::HOLD_MORE`.
    pub const HOLD_MORE: &str = "More: +minds/holdings";
    /// `tacsim::minds::words::HOLD_EMPTY`.
    pub const HOLD_EMPTY: &str = "You hold no mind licences.";
    /// `tacsim::minds::words::YOUR_QUOTE`.
    pub const YOUR_QUOTE: &str = "more seconds. Pay it with +minds/buy/pay";
    /// `tacsim::minds::words::QUOTE_EXPIRED`.
    pub const QUOTE_EXPIRED: &str = "Your quote has expired. Ask for a new one with +minds/buy";
    /// `tacsim::minds::words::YOUR_ESCROW`.
    pub const YOUR_ESCROW: &str =
        "Your payment is held in escrow. Take the licence with +minds/buy/complete";
    /// `tacsim::minds::words::ASK`.
    pub const ASK: &str = "Ask for a quote with +minds/buy";
    /// `tacsim::minds::words::NO_QUOTE`.
    pub const NO_QUOTE: &str = "Ask for one with +minds/buy";
    /// `tacsim::minds::words::PAYING`.
    pub const PAYING: &str = "Paying your quote on listing";
    /// `tacsim::minds::words::NO_PURCHASE`.
    pub const NO_PURCHASE: &str = "You have no paid purchase waiting on listing";
    /// `tacsim::minds::words::COMPLETING`.
    pub const COMPLETING: &str = "Taking the licence from listing";
    /// `tacsim::minds::words::REFUNDING`.
    pub const REFUNDING: &str = "Asking for your payment on listing";
    /// `tacsim::minds::words::HOLD_PAID`: the paid row's two-space indent.
    pub const HOLD_PAID: &str = "  licence ";
    /// `tacsim::minds::words::HOLD_WAIT_HEAD`.
    pub const HOLD_WAIT_HEAD: &str = "Purchases waiting:";
    /// `tacsim::minds::words::HOLD_WAIT`.
    pub const HOLD_WAIT: &str = "paid into escrow: +minds/buy/complete";
    /// `tacsim::minds::words::NO_LICENCE`.
    pub const NO_LICENCE: &str = "You hold no licence";
    /// `tacsim::minds::words::NOT_FOR_ORG`.
    pub const NOT_FOR_ORG: &str = "You do not act for that organisation.";
    /// `tacsim::minds::words::NO_RENEWAL`.
    pub const NO_RENEWAL: &str =
        "has no renewal offer waiting for you. Ask for one with +minds/renew";
    /// `tacsim::minds::words::RESELLING`.
    pub const RESELLING: &str = "Offering licence";
    /// `tacsim::minds::words::RESELL_TELL`.
    pub const RESELL_TELL: &str = "Once that is confirmed, they accept with +minds/resell/accept";
    /// `tacsim::minds::words::CHARTERING`.
    pub const CHARTERING: &str = "Offering filed mind";
    /// `tacsim::minds::words::CHARTER_TELL`.
    pub const CHARTER_TELL: &str = "Once that is confirmed, they accept with +minds/charter/accept";
    /// `tacsim::minds::words::NO_PLAYER`.
    pub const NO_PLAYER: &str = "No player is named";
    /// `tacsim::minds::words::BAD_PRICE`.
    pub const BAD_PRICE: &str = "price is in credits, from 0.01 to 1000000, such as price=12.50.";
    /// `tacsim::minds::words::RECALLING`.
    pub const RECALLING: &str = "Recalling filed revision";
    /// `tacsim::minds::words::REVOKING`.
    pub const REVOKING: &str = "Revoking licence";
    /// `tacsim::minds::words::REGISTRY_OFF`.
    pub const REGISTRY_OFF: &str = "Registry recall and revocation are not configured here.";
    /// `tacsim::minds::words::INSPECTING`.
    pub const INSPECTING: &str = "Inspecting hull #";
    /// `tacsim::minds::words::INSPECTION_OFF`.
    pub const INSPECTION_OFF: &str = "Registry inspection is not configured here.";
    /// `tacsim::minds::words::INSPECT_REFUSED`: the driver's one refusal, `mush_core::action::SUBMISSION_REFUSED_LINE`.
    pub const INSPECT_REFUSED: &str = "That did not go through.";
    /// `tacsim::minds::words::NO_SUCH_HULL`.
    pub const NO_SUCH_HULL: &str = "No such hull.";
    /// `tacsim::minds::words::FINING`.
    pub const FINING: &str = "Fining the holder of hull #";
    /// `tacsim::minds::words::FINE_OFF`.
    pub const FINE_OFF: &str = "Registry fines are not configured here.";
    /// `tacsim::minds::words::NO_SUCH_INSPECTION`.
    pub const NO_SUCH_INSPECTION: &str = "No such inspection.";
    /// `tacsim::minds::words::NOTHING_TO_FINE`.
    pub const NOTHING_TO_FINE: &str = "That inspection found nothing to fine.";
    /// `tacsim::minds::words::FINE_RANGE`.
    pub const FINE_RANGE: &str = "A fine is from 0.01cr to";
    /// `tacsim::minds::words::NO_FINE_HOLDER`.
    pub const NO_FINE_HOLDER: &str = "That hull has no holder this Registry can fine.";
    /// `tacsim::minds::words::ALREADY_FINED`.
    pub const ALREADY_FINED: &str = "That inspection has already been fined.";
    /// `tacsim::minds::words::REVOKING_PRINCIPAL`: shared with `REVOKING` (an allowed overlap).
    pub const REVOKING_PRINCIPAL: &str = "Revoking ";
    /// `tacsim::minds::words::PRINCIPALS_BARRED`.
    pub const PRINCIPALS_BARRED: &str = "The Registry has barred your mind licences until";
    /// `tacsim::minds::words::PRINCIPAL_REVOKED`.
    pub const PRINCIPAL_REVOKED: &str = "holds no new licence until";
    /// `tacsim::minds::words::NO_SUCH_PRINCIPAL`.
    pub const NO_SUCH_PRINCIPAL: &str = "No such character or organisation.";
    /// `tacsim::minds::words::AMBIGUOUS_PRINCIPAL`.
    pub const AMBIGUOUS_PRINCIPAL: &str =
        "That name is a character and an organisation; name the character as #<n>.";
    /// `tacsim::minds::words::NOTHING_TO_REVOKE`.
    pub const NOTHING_TO_REVOKE: &str = "That inspection found nothing to revoke for.";
    /// `tacsim::minds::words::CLAIM_FILED`.
    pub const CLAIM_FILED: &str = "Your claim is filed.";
    /// `tacsim::minds::words::CLAIMS_OFF`.
    pub const CLAIMS_OFF: &str = "Registry claims are not configured here.";
    /// `tacsim::minds::words::NO_CLAIMS`.
    pub const NO_CLAIMS: &str = "No claims are filed with the Registry.";
    /// `tacsim::minds::words::CLAIM_LINE`.
    pub const CLAIM_LINE: &str = ") claim ";
    /// `tacsim::minds::words::CLAIM_WORDS`.
    pub const CLAIM_WORDS: &str = "In their words:";
    /// `tacsim::minds::words::MORE_CLAIMS`.
    pub const MORE_CLAIMS: &str = "older claims are not shown.";
    /// `tacsim::minds::words::SEIZING`.
    pub const SEIZING: &str = "Seizing the core in hull #";
    /// `tacsim::minds::words::SEIZE_OFF`.
    pub const SEIZE_OFF: &str = "Registry seizure is not configured here.";
    /// `tacsim::minds::words::NOTHING_TO_SEIZE`.
    pub const NOTHING_TO_SEIZE: &str = "The latest inspection of that hull found nothing to seize.";
    /// `tacsim::minds::words::ALREADY_SEIZED`.
    pub const ALREADY_SEIZED: &str = "That inspection's core is already seized.";
    /// `tacsim::minds::words::SEIZED`.
    pub const SEIZED: &str = "It stays fitted and flies nothing; it may be forfeited after";
    /// `tacsim::minds::words::FORFEITING`.
    pub const FORFEITING: &str = "Forfeiting the core seized from hull #";
    /// `tacsim::minds::words::NO_SUCH_SEIZURE`.
    pub const NO_SUCH_SEIZURE: &str = "No such seizure.";
    /// `tacsim::minds::words::IN_HOLD`.
    pub const IN_HOLD: &str = "That seizure is still within its hold.";
    /// `tacsim::minds::words::FORFEITED`.
    pub const FORFEITED: &str = "The core seized from hull #";
    /// `tacsim::minds::words::FINED`.
    pub const FINED: &str = "The fine is levied when the Registry next settles.";
    /// `tacsim::minds::words::RENEW_HANDED`.
    pub const RENEW_HANDED: &str = "to the renewal service.";

    /// The named buyer's offers and accepts: `tacsim::minds::words::named`.
    pub mod named {
        /// `tacsim::minds::words::named::OFFER`.
        pub const OFFER: &str = "is offered to you for";
        /// `tacsim::minds::words::named::ACCEPT_HOW`.
        pub const ACCEPT_HOW: &str = "Accept it with +minds/resell/accept";
        /// `tacsim::minds::words::named::NO_OFFER`.
        pub const NO_OFFER: &str = "You have no offer for licence";
        /// `tacsim::minds::words::named::NO_OFFER_AT`.
        pub const NO_OFFER_AT: &str = "None of your offers for licence";
        /// `tacsim::minds::words::named::CHARTER_OFFER`.
        pub const CHARTER_OFFER: &str = "and its charter are offered to you for";
        /// `tacsim::minds::words::named::CHARTER_ACCEPT_HOW`.
        pub const CHARTER_ACCEPT_HOW: &str = "Accept it with +minds/charter/accept";
        /// `tacsim::minds::words::named::NO_CHARTER_OFFER`.
        pub const NO_CHARTER_OFFER: &str = "You have no offer for filed mind";
        /// `tacsim::minds::words::named::NO_CHARTER_OFFER_AT`.
        pub const NO_CHARTER_OFFER_AT: &str = "None of your offers for filed mind";
        /// `tacsim::minds::words::named::ACCEPTING`.
        pub const ACCEPTING: &str = "Accepting the offer for licence";
        /// `tacsim::minds::words::named::FUNDING_CHARTER`.
        pub const FUNDING_CHARTER: &str =
            "into escrow; the transfer settles once no sortie under it is out.";
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    /// One row of the table: the game constant's name (`named::` before a constant of the
    /// game's inline `named` module), the game's sentence, and the bot's needle for it.
    type Row = (&'static str, &'static str, &'static str);

    /// The rows of one family, read through the `game` and `bot` modules the caller brings
    /// into scope: the needle and the sentence are named by the same identifier, so a row
    /// cannot pair a needle with another constant's sentence.
    macro_rules! rows {
        ($prefix:literal; $($name:ident),+ $(,)?) => {
            vec![$((concat!($prefix, stringify!($name)), game::$name, bot::$name)),+]
        };
    }

    // The game modules' sources, for the census. Paths are relative to this file.
    const END_USER_RS: &str = include_str!("../../../tacsim/src/launch/end_user.rs");
    const RENEWAL_RS: &str = include_str!("../../../mush-market/src/mind_renewal/player.rs");
    const EXCURSION_RS: &str = include_str!("../../../tacsim/src/surface/excursion.rs");
    const DISPATCH_RS: &str = include_str!("../../../tacsim/src/surface/dispatch.rs");
    const ABSENT_RS: &str = include_str!("../../../tacsim/src/field/absent.rs");
    const WORDS_RS: &str = include_str!("../../../tacsim/src/minds/market_commands/words.rs");

    fn end_user_rows() -> Vec<Row> {
        use super::end_user as bot;
        use tacsim::launch::end_user as game;
        rows!("";
            END_USER_HELP,
            END_USER_NOT_OPEN,
            END_USER_UNLISTED,
            END_USER_LISTED,
            END_USER_SUSPENDED,
            END_USER_NO_CREDIT,
            END_USER_FUNDS,
            END_USER_PREVIEW,
            END_USER_DONE,
            END_USER_FAILED,
            END_USER_USAGE,
        )
    }

    fn renewal_rows() -> Vec<Row> {
        use super::renewal as bot;
        use mush_market::mind_renewal::player as game;
        rows!("";
            QUEUED_LINE,
            UNRESOLVED_LINE,
            RENEW_NOT_DUE,
            RENEWING,
            RENEW_OFFER,
            RENEW_HOW,
            RENEW_FUNDED,
            RENEW_PAYING,
            RENEW_DECLINING,
            RENEW_STOPPED,
        )
    }

    fn claim_rows() -> Vec<Row> {
        use super::claim as bot;
        use tacsim::surface::excursion as game;
        rows!("";
            CLAIM_STILL_OPEN,
            CLAIM_REFUSED,
            CLAIM_CANCELLED,
            CLAIM_NOT_OPEN,
        )
    }

    fn dispatch_rows() -> Vec<Row> {
        use super::dispatch as bot;
        use tacsim::surface::dispatch as game;
        rows!("";
            MIND_PENDING,
            MIND_NO_LICENCE,
            MIND_GOVERNED_DRAFT,
            MIND_NO_DRAFT,
            MIND_NO_CORE,
            MIND_SEATS_HELD,
            MIND_UNAVAILABLE,
            MIND_UNTITLED,
            MIND_NOT_FOR_ORG,
        )
    }

    fn absent_rows() -> Vec<Row> {
        use super::absent as bot;
        use tacsim::field::absent as game;
        rows!("";
            MIND_HAS_IT,
            TAKEN_HOME,
            BACK_IN_SEAT,
            WENT_HOME,
            LEFT_AT_RAMP,
            NOT_FIT,
            GONE_HOME,
        )
    }

    fn market_rows() -> Vec<Row> {
        let mut rows = {
            use super::market as bot;
            use tacsim::minds::words as game;
            rows!("";
                NOT_OPEN,
                BUSY,
                ACCOUNT,
                NO_REVISION,
                NOT_RATED,
                RATING_BUSY,
                RESERVING,
                FILING,
                NO_FILED,
                UPDATING,
                WITHDRAWING,
                ATTESTING,
                FORKING,
                RATING_UNAVAILABLE,
                COUNTER,
                EMPTY,
                HEAD,
                ROW,
                FOOT,
                MORE,
                NO_LISTING,
                AMBIGUOUS,
                DETAIL,
                TERMS_LINE,
                SPLIT,
                QUOTING,
                PUBLISHING,
                TERMS,
                BAD_FORM,
                BAD_ROYALTY,
                BAD_TRANSFER,
                HOLD_HEAD,
                HOLD_OWNER,
                HOLD_MORE,
                HOLD_EMPTY,
                YOUR_QUOTE,
                QUOTE_EXPIRED,
                YOUR_ESCROW,
                ASK,
                NO_QUOTE,
                PAYING,
                NO_PURCHASE,
                COMPLETING,
                REFUNDING,
                HOLD_PAID,
                HOLD_WAIT_HEAD,
                HOLD_WAIT,
                NO_LICENCE,
                NOT_FOR_ORG,
                NO_RENEWAL,
                RESELLING,
                RESELL_TELL,
                CHARTERING,
                CHARTER_TELL,
                NO_PLAYER,
                BAD_PRICE,
                RECALLING,
                REVOKING,
                REGISTRY_OFF,
                INSPECTING,
                INSPECTION_OFF,
                INSPECT_REFUSED,
                NO_SUCH_HULL,
                FINING,
                FINE_OFF,
                NO_SUCH_INSPECTION,
                NOTHING_TO_FINE,
                FINE_RANGE,
                NO_FINE_HOLDER,
                ALREADY_FINED,
                REVOKING_PRINCIPAL,
                PRINCIPALS_BARRED,
                PRINCIPAL_REVOKED,
                NO_SUCH_PRINCIPAL,
                AMBIGUOUS_PRINCIPAL,
                NOTHING_TO_REVOKE,
                CLAIM_FILED,
                CLAIMS_OFF,
                NO_CLAIMS,
                CLAIM_LINE,
                CLAIM_WORDS,
                MORE_CLAIMS,
                SEIZING,
                SEIZE_OFF,
                NOTHING_TO_SEIZE,
                ALREADY_SEIZED,
                SEIZED,
                FORFEITING,
                NO_SUCH_SEIZURE,
                IN_HOLD,
                FORFEITED,
                FINED,
                RENEW_HANDED,
            )
        };
        rows.extend({
            use super::market::named as bot;
            use tacsim::minds::words::named as game;
            rows!("named::";
                OFFER,
                ACCEPT_HOW,
                NO_OFFER,
                NO_OFFER_AT,
                CHARTER_OFFER,
                CHARTER_ACCEPT_HOW,
                NO_CHARTER_OFFER,
                NO_CHARTER_OFFER_AT,
                ACCEPTING,
                FUNDING_CHARTER,
            )
        });
        rows
    }

    // ---- What the census does not ask a row for, each with its reason.

    /// `end_user.rs`.
    const END_USER_EXCLUDED: &[(&str, &str)] = &[(
        "END_USER_VERB",
        "the verb a player types (+enduser), not a sentence the game prints",
    )];
    /// `excursion.rs`: its `&str` constants other than the claim sentences.
    const EXCURSION_EXCLUDED: &[(&str, &str)] = &[
        ("EXCURSION_FAMILY", "a store family name, never printed"),
        ("TV_REC_PREFIX", "a store key prefix, never printed"),
    ];
    /// `words.rs`.
    const WORDS_EXCLUDED: &[(&str, &str)] = &[(
        "REFUSED",
        "the typed refusal's code word for a usage refusal; the line prints the usage text, \
         never this word",
    )];

    // ---- The pairs allowed to overlap within a family: (row, other row, why).

    /// The market family's overlaps. Both are a staged request line that opens with the same
    /// verb as another and has no other fragment of its own; a reader tells them apart by
    /// testing the longer sentence's needle first.
    const MARKET_OVERLAPS: &[(&str, &str, &str)] = &[
        (
            "FILING",
            "UPDATING",
            "\"Filing {reference} as {charter}.\": its only other fragment is \"as\"; \
             UPDATING's needle (\"on the line of filed mind\") is checked first",
        ),
        (
            "REVOKING_PRINCIPAL",
            "REVOKING",
            "\"Revoking {who} for {days} IC days.\": \"for\" and \"IC days.\" are shared too \
             (SEIZED); REVOKING's needle (\"Revoking licence\") is checked first",
        ),
    ];

    // ---- The checks.

    /// The literal runs of a template: the text between its `{placeholder}`s.
    fn literal_runs(template: &str) -> Vec<&str> {
        let mut runs = Vec::new();
        let mut rest = template;
        while let Some(open) = rest.find('{') {
            runs.push(&rest[..open]);
            match rest[open..].find('}') {
                Some(close) => rest = &rest[open + close + 1..],
                None => {
                    rest = &rest[open..];
                    break;
                }
            }
        }
        runs.push(rest);
        runs
    }

    /// **In step**: every needle is a non-blank fragment of one literal run of its sentence.
    fn in_step(family: &str, rows: &[Row]) {
        let mut wrong = Vec::new();
        for &(name, sentence, needle) in rows {
            let found = literal_runs(sentence)
                .iter()
                .any(|run| run.contains(needle));
            if needle.trim().is_empty() || !found {
                wrong.push(format!(
                    "{family}::{name}: needle {needle:?} in {sentence:?}"
                ));
            }
        }
        assert!(
            wrong.is_empty(),
            "the bot's needles are not literal fragments of the game's sentences \
             (a game sentence changed; update `mine::minds`):\n{}",
            wrong.join("\n")
        );
    }

    /// **Distinct**: no needle is found in another member's sentence, but the listed pairs,
    /// and every listed pair still overlaps.
    fn distinct(family: &str, rows: &[Row], overlaps: &[(&str, &str, &str)]) {
        let mut wrong = Vec::new();
        for &(name, _, needle) in rows {
            for &(other, sentence, _) in rows {
                let listed = overlaps.iter().any(|&(a, b, _)| a == name && b == other);
                if other != name && sentence.contains(needle) && !listed {
                    wrong.push(format!(
                        "{family}::{name}'s needle {needle:?} is in {family}::{other}'s sentence"
                    ));
                }
            }
        }
        for &(a, b, _) in overlaps {
            let needle = rows.iter().find(|row| row.0 == a).map(|row| row.2);
            let sentence = rows.iter().find(|row| row.0 == b).map(|row| row.1);
            match (needle, sentence) {
                (Some(needle), Some(sentence)) if sentence.contains(needle) => {}
                _ => wrong.push(format!(
                    "{family}: listed overlap ({a}, {b}) no longer overlaps"
                )),
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// `line` without a leading `pub`, `pub(crate)`, `pub(super)` or `pub(self)`.
    fn without_visibility(line: &str) -> &str {
        ["pub(crate) ", "pub(super) ", "pub(self) ", "pub "]
            .iter()
            .find_map(|prefix| line.strip_prefix(prefix))
            .unwrap_or(line)
    }

    /// The name of a `mod name {` line (any visibility), else `None`.
    fn inline_module(line: &str) -> Option<&str> {
        without_visibility(line)
            .strip_prefix("mod ")?
            .strip_suffix(" {")
    }

    fn is_const_name(name: &str) -> bool {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    }

    /// The name of a `const NAME: &str` (or `&'static str`) declaration, any visibility.
    fn str_const(line: &str) -> Option<&str> {
        let (name, ty) = without_visibility(line)
            .strip_prefix("const ")?
            .split_once(": ")?;
        (is_const_name(name) && (ty.starts_with("&str") || ty.starts_with("&'static str")))
            .then_some(name)
    }

    /// The name of a `pub use path::THING as NAME;` re-export of a constant.
    fn reexport(line: &str) -> Option<&str> {
        let (_, name) = line
            .strip_prefix("pub use ")?
            .strip_suffix(';')?
            .rsplit_once(" as ")?;
        is_const_name(name).then_some(name)
    }

    /// Every `&str` constant a game module's source declares at module level, and every
    /// constant it re-exports under a new name: `NAME` at column 0, or `module::NAME` four
    /// columns into a top-level inline module (`words.rs`' `named`). A `tests` module and a
    /// constant local to a function body are not read.
    fn declared(source: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut inner: Option<&str> = None;
        for line in source.lines() {
            if line == "}" {
                inner = None;
                continue;
            }
            if let Some(name) = inline_module(line) {
                inner = Some(name);
                continue;
            }
            let (body, prefix) = match inner {
                None => (line, String::new()),
                Some("tests") => continue,
                Some(module) => match line.strip_prefix("    ") {
                    Some(body) => (body, format!("{module}::")),
                    None => continue,
                },
            };
            if body.starts_with(' ') {
                continue;
            }
            if let Some(name) = str_const(body).or_else(|| reexport(body)) {
                out.push(format!("{prefix}{name}"));
            }
        }
        out
    }

    /// **Complete**: every constant `declared` finds in `source` has a row or an exclusion,
    /// and every row and exclusion names a constant still declared there.
    fn census(file: &str, source: &str, rows: &[Row], excluded: &[(&str, &str)]) {
        let declared = declared(source);
        let known: BTreeSet<&str> = rows
            .iter()
            .map(|row| row.0)
            .chain(excluded.iter().map(|exclusion| exclusion.0))
            .collect();
        let missing: Vec<&str> = declared
            .iter()
            .map(String::as_str)
            .filter(|name| !known.contains(name))
            .collect();
        assert!(
            missing.is_empty(),
            "{file} declares sentence constants the bot's table has no row for: {missing:?}. \
             Add a needle to `mine::minds` (or an exclusion that says why)."
        );
        let stale: Vec<&str> = known
            .iter()
            .copied()
            .filter(|name| !declared.iter().any(|d| d == name))
            .collect();
        assert!(
            stale.is_empty(),
            "the bot's table has rows for constants {file} no longer declares: {stale:?}"
        );
    }

    /// The census reader itself, on a made-up module.
    #[test]
    fn the_census_reads_module_level_str_constants() {
        let source = "pub const A: &str = \"a\";\n\
                      pub(crate) const B_2: &'static str =\n    \"b\";\n\
                      const C: &str = \"c\";\n\
                      pub const N: u32 = 1;\n\
                      pub use crate::x::A as D;\n\
                      fn f() {\n    const LOCAL: &str = \"l\";\n}\n\
                      pub(crate) mod named {\n    pub const E: &str = \"e\";\n}\n\
                      /// pub const F: &str = \"f\";\n\
                      #[cfg(test)]\nmod tests {\n    pub const G: &str = \"g\";\n}\n";
        assert_eq!(declared(source), ["A", "B_2", "C", "D", "named::E"]);
        assert_eq!(literal_runs("{a} b {c:<4}d."), ["", " b ", "d."]);
    }

    #[test]
    fn end_user_needles_are_in_step() {
        in_step("end_user", &end_user_rows());
    }
    #[test]
    fn end_user_census() {
        census(
            "end_user.rs",
            END_USER_RS,
            &end_user_rows(),
            END_USER_EXCLUDED,
        );
    }
    #[test]
    fn end_user_needles_are_distinct() {
        distinct("end_user", &end_user_rows(), &[]);
    }

    #[test]
    fn renewal_needles_are_in_step() {
        in_step("renewal", &renewal_rows());
    }
    #[test]
    fn renewal_census() {
        census("mind_renewal/player.rs", RENEWAL_RS, &renewal_rows(), &[]);
    }
    #[test]
    fn renewal_needles_are_distinct() {
        distinct("renewal", &renewal_rows(), &[]);
    }

    #[test]
    fn claim_needles_are_in_step() {
        in_step("claim", &claim_rows());
    }
    #[test]
    fn claim_census() {
        census(
            "excursion.rs",
            EXCURSION_RS,
            &claim_rows(),
            EXCURSION_EXCLUDED,
        );
    }
    #[test]
    fn claim_needles_are_distinct() {
        distinct("claim", &claim_rows(), &[]);
    }

    #[test]
    fn dispatch_needles_are_in_step() {
        in_step("dispatch", &dispatch_rows());
    }
    #[test]
    fn dispatch_census() {
        census("dispatch.rs", DISPATCH_RS, &dispatch_rows(), &[]);
    }
    #[test]
    fn dispatch_needles_are_distinct() {
        distinct("dispatch", &dispatch_rows(), &[]);
    }

    #[test]
    fn absent_needles_are_in_step() {
        in_step("absent", &absent_rows());
    }
    #[test]
    fn absent_census() {
        census("absent.rs", ABSENT_RS, &absent_rows(), &[]);
    }
    #[test]
    fn absent_needles_are_distinct() {
        distinct("absent", &absent_rows(), &[]);
    }

    #[test]
    fn market_needles_are_in_step() {
        in_step("market", &market_rows());
    }
    #[test]
    fn market_census() {
        census("words.rs", WORDS_RS, &market_rows(), WORDS_EXCLUDED);
    }
    #[test]
    fn market_needles_are_distinct() {
        distinct("market", &market_rows(), MARKET_OVERLAPS);
    }
}

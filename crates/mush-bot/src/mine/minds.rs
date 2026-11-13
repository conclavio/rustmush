//! **The mind market's sentences, as the bot reads them** (doc 44 MW.5; doc 17g C4 item 3,
//! "`cargo test -p mush-bot` green on every new sentence").
//!
//! One `pub const` needle per player-facing sentence constant of seven game modules, grouped in
//! one module per family and named exactly as the game's constant is named. A needle is a
//! literal fragment of the game's sentence: it never crosses a `{placeholder}`, so it is found
//! in the filled line whatever the fill, and it is chosen to tell its sentence from every other
//! sentence in the table (the longest placeholder-free fragment, unless that fragment is
//! shared).
//!
//! This is the table. The market's walk reads it (doc 44 MW.19: `super::lane::market`, with the
//! [`walk`] family for what no censused constant holds), and nothing here is a game-crate
//! dependency: the needles are the bot's own text. The tests
//! below hold the table to the game through the crate's development dependencies (`tacsim`,
//! `mush-market`), in the role [`super::lines`]' tests play for the mining loop:
//!
//! * **in step**: every needle is a literal fragment of the game constant it names, so a
//!   reworded sentence fails `cargo test -p mush-bot`;
//! * **complete**: a census reads each game module's source (`include_str!`) and fails naming
//!   any `&str` constant declared there with no row here and no listed exclusion, and fails
//!   naming any other `const` or `static` line whose type mentions `str` that it cannot
//!   attribute (one in a function or an `impl`, two modules deep, a table of strings), so a
//!   sentence constant added to one of these modules fails the bot's tests until the bot
//!   learns it;
//! * **distinct**: within a family and across all seven, no needle is found in another row's
//!   sentence, except the pairs listed in the test module's overlap tables, each with its
//!   reason.
//!
//! **What the census does not see.** It reads constants, not sentences a function builds. The
//! known ones in these modules: `mind_renewal::player`'s status details (the `format!` in
//! `personal_status_line` and `house_status_line`, "Price: … minor units of …") and
//! `stop_reason`'s reasons, the fill of `RENEW_STOPPED`; `field::absent`'s `Why::clause`, the
//! fill of `GONE_HOME`; `excursion`'s `word`/`words` renderings, which fill other modules'
//! sentences. A new sentence written inline in a function, rather than as a constant, is not
//! caught.
//!
//! **Sentences that are the same text.** `dispatch::MIND_NO_LICENCE` and `market::NO_LICENCE`
//! ("You hold no licence {licence}."), and `dispatch::MIND_NOT_FOR_ORG`, which the game
//! re-exports from `market::NOT_FOR_ORG`. A reader that must tell them apart does it by the
//! command it typed, not by the line.
//!
//! **Partial needles** ([`PARTIAL`]). Seven sentences have no placeholder-free fragment that
//! is both long and their own: two staged lines that open with another's verb, three table
//! rows, and two lines whose every fragment is short (`DETAIL`, `CLAIM_LINE`). Their needle alone does not identify the sentence; each needle's doc says
//! what a reader must use beside it, and the rule for a reader (MW.19) is: test a longer
//! sentence's needle before a partial one that it contains (`UPDATING` before `FILING`,
//! `REVOKING` before `REVOKING_PRINCIPAL`), and never collapse runs of spaces in a line before
//! looking for `ROW`, `HOLD_OWNER` or `HOLD_PAID`.
//!
//! One game constant has no row: `market::INSPECT_REFUSED` is the engine's generic refused line
//! (`mush_core::action::SUBMISSION_REFUSED_LINE`, "That did not go through. Nothing changed."),
//! printed for every refused submission, so no needle of it could name this family.

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
    /// `mush_market::mind_renewal::player::STATUS_NONE`.
    pub const STATUS_NONE: &str = "No renewal offer has been requested.";
    /// `mush_market::mind_renewal::player::STATUS_DRAFTED`.
    pub const STATUS_DRAFTED: &str = "A renewal offer awaits your explicit acceptance or decline.";
    /// `mush_market::mind_renewal::player::STATUS_FUNDED`.
    pub const STATUS_FUNDED: &str = "Payment is held; completion is pending.";
    /// `mush_market::mind_renewal::player::STATUS_GRANTED`.
    pub const STATUS_GRANTED: &str = "Renewal completed.";
    /// `mush_market::mind_renewal::player::STATUS_CLOSED`.
    pub const STATUS_CLOSED: &str =
        "The last renewal closed without a new term. You may explicitly request a retry.";
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
    /// `tacsim::field::absent::ADMITTED_AWAY` (doc 45 cut AP.5).
    pub const ADMITTED_AWAY: &str =
        "while you were away: its mind has the controls, makes for its carrier's ramp";
    /// `tacsim::field::absent::NOT_FIT`.
    pub const NOT_FIT: &str = "its mind keeps the controls.";
    /// `tacsim::field::absent::GONE_HOME`.
    pub const GONE_HOME: &str = "breaks off for its carrier's ramp under its mind:";
    /// `tacsim::field::absent::MIND_HAS_IT_ON` (doc 45 unit AP.6).
    pub const MIND_HAS_IT_ON: &str = "its mind has the controls and keeps it on ";
    /// `tacsim::field::absent::TAKEN_ON` (doc 45 unit AP.6).
    pub const TAKEN_ON: &str = "passed to its mind, which flies it on ";
    /// `tacsim::field::absent::ADMITTED_ON` (doc 45 unit AP.6).
    pub const ADMITTED_ON: &str = "while you were away: its mind has the controls and flies it on ";
    /// `tacsim::field::absent::STAYS_ON` (doc 45 unit AP.6).
    pub const STAYS_ON: &str = "passes to its mind, which flies it on ";
    /// `tacsim::field::absent::UNATTENDED` (doc 45 unit AP.7).
    pub const UNATTENDED: &str =
        "with nobody in its seat: its mind has the controls, makes for its carrier's ramp";
    /// `tacsim::field::absent::UNATTENDED_ON` (doc 45 unit AP.7).
    pub const UNATTENDED_ON: &str =
        "with nobody in its seat: its mind has the controls and flies it on ";
}

/// `+ship/mind`, a mind sealed aboard a ship (doc 44 MW.13): `tacsim::port::ship_mind`.
pub mod ship_mind {
    /// `tacsim::port::ship_mind::MIND_USAGE`.
    pub const MIND_USAGE: &str = "Usage: +ship/mind | +ship/mind filed:<licence>";
    /// `tacsim::port::ship_mind::MIND_NONE`.
    pub const MIND_NONE: &str = "If you lose your link, the house default takes her home.";
    /// `tacsim::port::ship_mind::MIND_ADMITTING`.
    pub const MIND_ADMITTING: &str = "The Registry is admitting licence ";
    /// `tacsim::port::ship_mind::MIND_ABOARD`.
    pub const MIND_ABOARD: &str =
        "It has her helm and sensors while her crew has no link, until she next docks at a port.";
    /// `tacsim::port::ship_mind::MIND_UNFIT`.
    pub const MIND_UNFIT: &str = ", and no longer fits her; its claim is being closed.";
    /// `tacsim::port::ship_mind::MIND_CLOSING`.
    pub const MIND_CLOSING: &str = "The claim on the mind last sealed aboard the ";
    /// `tacsim::port::ship_mind::MIND_WAS_REFUSED`.
    pub const MIND_WAS_REFUSED: &str =
        "The Registry did not admit the last mind you asked to seal aboard the ";
    /// `tacsim::port::ship_mind::MIND_BERTH`.
    pub const MIND_BERTH: &str = " at her berth in a port, before she undocks.";
    /// `tacsim::port::ship_mind::MIND_NO_CORE`.
    pub const MIND_NO_CORE: &str =
        " carries no core; her drive ROM flies a filed plan and decides nothing.";
    /// `tacsim::port::ship_mind::MIND_ALREADY`.
    pub const MIND_ALREADY: &str = "; +ship/mind off unseals it at her berth.";
    /// `tacsim::port::ship_mind::MIND_PASSAGE`.
    pub const MIND_PASSAGE: &str =
        "'s last passage is still being written up; try again in a moment.";
    /// `tacsim::port::ship_mind::MIND_DRAFT`.
    pub const MIND_DRAFT: &str = "an unfiled draft cannot be sealed aboard a ship.";
    /// `tacsim::port::ship_mind::MIND_LAPSED`.
    pub const MIND_LAPSED: &str = "; nothing is sealed aboard.";
    /// `tacsim::port::ship_mind::MIND_PREVIEW`.
    pub const MIND_PREVIEW: &str =
        "and holds a seat of the licence until she next docks at a port. Add go to seal it.";
    /// `tacsim::port::ship_mind::MIND_NOT_SEALED`.
    pub const MIND_NOT_SEALED: &str = " has no mind sealed aboard to unseal.";
    /// `tacsim::port::ship_mind::MIND_NOT_YOURS`.
    pub const MIND_NOT_YOURS: &str = " was sealed by someone else; they unseal it.";
    /// `tacsim::port::ship_mind::MIND_NOT_YET`.
    pub const MIND_NOT_YET: &str = "'s mind claim yet; try again in a moment.";
    /// `tacsim::port::ship_mind::MIND_OFF_PREVIEW`.
    pub const MIND_OFF_PREVIEW: &str =
        ": its claim closes and the licence's seat is free. Add go to unseal it.";
    /// `tacsim::port::ship_mind::MIND_SEALED`.
    pub const MIND_SEALED: &str = "The Registry admits the claim: ";
    /// `tacsim::port::ship_mind::MIND_REFUSED`.
    pub const MIND_REFUSED: &str =
        "The Registry would not admit her mind's claim: nothing is sealed aboard the ";
    /// `tacsim::port::ship_mind::MIND_UNSEALED`.
    pub const MIND_UNSEALED: &str = " is unsealed and its claim is closed.";
    /// `tacsim::port::ship_mind::MIND_VOIDED`.
    pub const MIND_VOIDED: &str = " no longer fits her (";
    /// `tacsim::port::ship_mind::MIND_UNWIRED`.
    pub const MIND_UNWIRED: &str = "Minds cannot be sealed aboard a ship here.";
    /// `tacsim::port::ship_mind::MIND_HOLDS_HER`.
    pub const MIND_HOLDS_HER: &str =
        ", and she is held as she is until it is unsealed: +ship/mind off, at her berth.";
}

/// A posture practised in the simulator (doc 45 unit AP.10): `tacsim::match_::practice`.
pub mod practice {
    /// `tacsim::match_::practice::PRACTICE_SET`.
    pub const PRACTICE_SET: &str =
        "for this match. MIND AWAY lets its mind fly it on that word while you watch.";
    /// `tacsim::match_::practice::PRACTICE_SET_AWAY`.
    pub const PRACTICE_SET_AWAY: &str = ": its mind flies it on that word from its next think.";
    /// `tacsim::match_::practice::PRACTICE_READ_AWAY`.
    pub const PRACTICE_READ_AWAY: &str =
        ", and its mind is flying it on that word. MIND BACK takes the controls again.";
    /// `tacsim::match_::practice::PRACTICE_AWAY`.
    pub const PRACTICE_AWAY: &str = "as if you were away. MIND BACK takes the controls again.";
    /// `tacsim::match_::practice::PRACTICE_ALREADY_AWAY`.
    pub const PRACTICE_ALREADY_AWAY: &str =
        "'s mind already has the controls. MIND BACK takes them again.";
    /// `tacsim::match_::practice::PRACTICE_BACK`.
    pub const PRACTICE_BACK: &str = "You take the controls again: ";
    /// `tacsim::match_::practice::PRACTICE_NOT_AWAY`.
    pub const PRACTICE_NOT_AWAY: &str = "You already hold ";
    /// `tacsim::match_::practice::PRACTICE_ONLY`.
    pub const PRACTICE_ONLY: &str = "MIND AWAY and MIND BACK are the simulator's.";
    /// `tacsim::match_::practice::PRACTICE_CHANNELS`.
    pub const PRACTICE_CHANNELS: &str =
        "Channels: the mind holds every one and flies the hull on its posture;";
    /// `tacsim::match_::practice::PRACTICE_SEAT_REFUSED`.
    pub const PRACTICE_SEAT_REFUSED: &str =
        "the seat gives no order while the mind flies the hull;";
    /// `tacsim::match_::practice::PRACTICE_STATUS_AWAY`.
    pub const PRACTICE_STATUS_AWAY: &str = "its mind has the controls; MIND BACK takes them again";
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
    /// `tacsim::minds::words::FILING`. Partial: also the opening of `UPDATING`; a reader takes
    /// a line that starts with it, does not hold `UPDATING`'s needle, and has " as " after it.
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
    /// `tacsim::minds::words::ROW`. Partial: a table row's gap before `class`; a reader takes it
    /// only from a line after `HEAD`, and must not collapse runs of spaces first.
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
    /// `tacsim::minds::words::DETAIL`. Partial: no longer fragment exists; a reader takes a line
    /// that also starts with "Listing " and holds ", class ".
    pub const DETAIL: &str = "and above.";
    /// `tacsim::minds::words::TERMS_LINE`.
    pub const TERMS_LINE: &str = "days of Grace; the licence";
    /// `tacsim::minds::words::SPLIT`.
    pub const SPLIT: &str = "charter owner the rest.";
    /// `tacsim::minds::words::QUOTING`.
    pub const QUOTING: &str = "Asking for a quote on listing";
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
    /// `tacsim::minds::words::HOLD_OWNER`. Partial: a holdings row's indent; a reader takes it
    /// only from a line that starts with it, after `HOLD_HEAD`, spaces not collapsed.
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
    /// `tacsim::minds::words::HOLD_PAID`. Partial: a holdings row's indent; a reader takes it
    /// only from a line that starts with it, after `HOLD_HEAD`, spaces not collapsed.
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
    /// `tacsim::minds::words::REVOKING_PRINCIPAL`. Partial: also the opening of `REVOKING`; a
    /// reader takes a line that starts with it, does not hold `REVOKING`'s needle, and ends
    /// " IC days.".
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
    /// `tacsim::minds::words::CLAIM_LINE`. Partial: no longer fragment exists; a reader takes a
    /// line that also holds ": hulls ", in the answer to `@registry/claims`.
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

/// **What the market's walk reads that the six families do not hold** (doc 44 unit MW.19): the
/// engine's two words for a submitted step, the Registry's notices about a rating, the
/// simulator's words about a chosen mind, and the launch's word for a licence that did not
/// seal. Most of these are written inline where the game prints them and are not constants, so
/// no census can see them; the test below holds each needle to the game's own source text (or
/// to its constant, where it has one), so a reworded sentence still fails `cargo test -p
/// mush-bot`.
pub mod walk {
    /// The engine's answer to a line it took for the world to commit: `mush-core`,
    /// `engine.rs`. A whole line; a reader takes a line that is exactly it.
    pub const QUEUED: &str = "Queued.";
    /// The world task's answer when that commit landed: `mush-server`, `world_task.rs`. A
    /// whole line; a reader takes a line that is exactly it.
    pub const DONE: &str = "Done.";
    /// `mush_core::action::SUBMISSION_REFUSED_LINE`: the commit was refused.
    pub const REFUSED: &str = "That did not go through. Nothing changed.";
    /// `@mind/rate`'s answer: `tacsim`, `minds/bulk/registry_service.rs`.
    pub const RATING_QUEUED: &str = "Registry preparation queued.";
    /// The Registry admitted the rating: the same file.
    pub const RATING_RETAINED: &str = "Registry work retained and queued.";
    /// The Registry's last word on a rating, the same for one that left a rating and one that
    /// failed: the same file.
    pub const RATING_FINISHED: &str = "Registry work finished.";
    /// The Registry refused to prepare the rating: the same file (its reason follows).
    pub const RATING_REFUSED: &str = "Registry preparation refused";
    /// The Registry did not admit the rating: the same file.
    pub const RATING_NOT_ADMITTED: &str = "Registry preparation was not admitted.";
    /// `SIM MIND`'s answer to a choice: `tacsim`, `match_/lobby.rs`.
    pub const SIM_CARRY: &str = "Your slot will carry ";
    /// The seat's line naming the mind sealed for the match, before `Good hunting.`: the same
    /// file.
    pub const SIM_SEALED: &str = "Sealed for this sortie: ";
    /// The cockpit's `MIND` head for a core that thinks: `tacsim`, `match_/cockpit/notices.rs`.
    pub const COCKPIT_MIND: &str = ", sealed for this sortie; it thinks at Class ";
    /// `EXIT` from a running match: the same file (`EXIT_FINAL`).
    pub const SIM_ON_ITS_OWN: &str =
        "is on its own for the rest of the match; you cannot take it back.";
    /// `SIM REPORT`'s line for the pilot's own mind: `tacsim`, `match_/report.rs`.
    pub const SIM_YOUR_MIND: &str = "Your mind: ";
    /// `SIM REPORT`'s line under it, when the mind flew the tank: the same file.
    pub const SIM_IT_FLEW: &str = "It flew your tank";
    /// `tacsim::port::tv::LAUNCHED_DOCTRINE`: the licence named at the launch had lapsed or
    /// been revoked, and the rig flies the ROM.
    pub const LAUNCHED_DOCTRINE: &str = "flies Standard Doctrine sealed aboard.";
}

/// **The partial needles**: rows (`family::NAME`) whose needle alone does not identify the
/// sentence. Each needle's doc says what a reader must use beside it. A test holds this list to
/// the table: every row whose needle is shorter than [`PARTIAL_BELOW`] characters (trimmed) is
/// listed, and every listed row exists.
pub const PARTIAL: &[&str] = &[
    "market::FILING",
    "market::ROW",
    "market::DETAIL",
    "market::HOLD_OWNER",
    "market::HOLD_PAID",
    "market::REVOKING_PRINCIPAL",
    "market::CLAIM_LINE",
];

/// The needle length, in characters after trimming, below which a row must be in [`PARTIAL`].
pub const PARTIAL_BELOW: usize = 12;

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
    const PRACTICE_RS: &str = include_str!("../../../tacsim/src/match_/practice.rs");
    const WORDS_RS: &str = include_str!("../../../tacsim/src/minds/market_commands/words.rs");
    const SHIP_MIND_RS: &str = include_str!("../../../tacsim/src/port/ship_mind.rs");

    fn ship_mind_rows() -> Vec<Row> {
        use super::ship_mind as bot;
        use tacsim::port::ship_mind as game;
        rows!("";
            MIND_USAGE,
            MIND_NONE,
            MIND_ADMITTING,
            MIND_ABOARD,
            MIND_UNFIT,
            MIND_CLOSING,
            MIND_WAS_REFUSED,
            MIND_BERTH,
            MIND_NO_CORE,
            MIND_ALREADY,
            MIND_PASSAGE,
            MIND_DRAFT,
            MIND_LAPSED,
            MIND_PREVIEW,
            MIND_NOT_SEALED,
            MIND_NOT_YOURS,
            MIND_NOT_YET,
            MIND_OFF_PREVIEW,
            MIND_SEALED,
            MIND_REFUSED,
            MIND_UNSEALED,
            MIND_VOIDED,
            MIND_UNWIRED,
            MIND_HOLDS_HER,
        )
    }

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
            STATUS_NONE,
            STATUS_DRAFTED,
            STATUS_FUNDED,
            STATUS_GRANTED,
            STATUS_CLOSED,
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
            ADMITTED_AWAY,
            GONE_HOME,
            MIND_HAS_IT_ON,
            TAKEN_ON,
            ADMITTED_ON,
            STAYS_ON,
            UNATTENDED,
            UNATTENDED_ON,
        )
    }

    fn practice_rows() -> Vec<Row> {
        use super::practice as bot;
        use tacsim::match_::practice as game;
        rows!("";
            PRACTICE_SET,
            PRACTICE_SET_AWAY,
            PRACTICE_READ_AWAY,
            PRACTICE_AWAY,
            PRACTICE_ALREADY_AWAY,
            PRACTICE_BACK,
            PRACTICE_NOT_AWAY,
            PRACTICE_ONLY,
            PRACTICE_STATUS_AWAY,
            PRACTICE_CHANNELS,
            PRACTICE_SEAT_REFUSED,
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

    /// Every family's rows, each name qualified as `family::NAME`, for the checks across the
    /// whole table.
    fn all_rows() -> Vec<(String, &'static str, &'static str)> {
        let families: [(&str, Vec<Row>); 8] = [
            ("end_user", end_user_rows()),
            ("renewal", renewal_rows()),
            ("claim", claim_rows()),
            ("dispatch", dispatch_rows()),
            ("absent", absent_rows()),
            ("market", market_rows()),
            ("ship_mind", ship_mind_rows()),
            ("practice", practice_rows()),
        ];
        families
            .into_iter()
            .flat_map(|(family, rows)| {
                rows.into_iter().map(move |(name, sentence, needle)| {
                    (format!("{family}::{name}"), sentence, needle)
                })
            })
            .collect()
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
    /// `ship_mind.rs`.
    const SHIP_MIND_EXCLUDED: &[(&str, &str)] = &[(
        "MIND_SWITCH",
        "the switch a player types (+ship/mind), not a sentence the game prints",
    )];
    /// `words.rs`.
    const WORDS_EXCLUDED: &[(&str, &str)] = &[
        (
            "REFUSED",
            "the typed refusal's code word for a usage refusal; the line prints the usage \
             text, never this word",
        ),
        (
            "INSPECT_REFUSED",
            "the engine's generic refused line (mush_core::action::SUBMISSION_REFUSED_LINE), \
             printed for every refused submission; no needle of it names this family",
        ),
    ];

    // ---- The pairs allowed to overlap: (row, other row, why), names as `family::NAME`.

    /// Within the market family: a staged request line that opens with the same verb as
    /// another and has no other fragment of its own. Both are in [`super::PARTIAL`]; the
    /// reader's rule (the module doc) is to test the other row's needle first.
    const MARKET_OVERLAPS: &[(&str, &str, &str)] = &[
        (
            "market::FILING",
            "market::UPDATING",
            "\"Filing {reference} as {charter}.\": its only other fragment is \"as\"",
        ),
        (
            "market::REVOKING_PRINCIPAL",
            "market::REVOKING",
            "\"Revoking {who} for {days} IC days.\": \"for\" and \"IC days.\" are shared too \
             (SEIZED)",
        ),
    ];

    /// Across families: the same sentence in two modules. A reader tells them apart by the
    /// command it typed.
    const CROSS_OVERLAPS: &[(&str, &str, &str)] = &[
        (
            "dispatch::MIND_NO_LICENCE",
            "market::NO_LICENCE",
            "the same text, \"You hold no licence {licence}.\", for a launch's mind= and for +minds",
        ),
        (
            "market::NO_LICENCE",
            "dispatch::MIND_NO_LICENCE",
            "the same text, the other way round",
        ),
        (
            "dispatch::MIND_NOT_FOR_ORG",
            "market::NOT_FOR_ORG",
            "the game re-exports market::NOT_FOR_ORG as dispatch::MIND_NOT_FOR_ORG",
        ),
        (
            "market::NOT_FOR_ORG",
            "dispatch::MIND_NOT_FOR_ORG",
            "the same constant, the other way round",
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

    /// **Distinct**: no needle is found in another row's sentence, but the listed pairs, and
    /// every listed pair still overlaps. Names are as `rows` spells them.
    fn distinct<N: AsRef<str>>(
        rows: &[(N, &'static str, &'static str)],
        overlaps: &[(&str, &str, &str)],
    ) {
        let mut wrong = Vec::new();
        for (name, _, needle) in rows {
            let name = name.as_ref();
            for (other, sentence, _) in rows {
                let other = other.as_ref();
                let listed = overlaps.iter().any(|&(a, b, _)| a == name && b == other);
                if other != name && sentence.contains(*needle) && !listed {
                    wrong.push(format!(
                        "{name}'s needle {needle:?} is in {other}'s sentence {sentence:?}"
                    ));
                }
            }
        }
        for &(a, b, _) in overlaps {
            let needle = rows.iter().find(|row| row.0.as_ref() == a).map(|row| row.2);
            let sentence = rows.iter().find(|row| row.0.as_ref() == b).map(|row| row.1);
            match (needle, sentence) {
                (Some(needle), Some(sentence)) if sentence.contains(needle) => {}
                _ => wrong.push(format!("listed overlap ({a}, {b}) no longer overlaps")),
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    /// `family::NAME` rows of a family, for the per-family distinct checks.
    fn qualified(family: &str, rows: Vec<Row>) -> Vec<(String, &'static str, &'static str)> {
        rows.into_iter()
            .map(|(name, sentence, needle)| (format!("{family}::{name}"), sentence, needle))
            .collect()
    }

    // ---- The census reader.

    /// What a census reads from a game module's source.
    #[derive(Debug, Default, PartialEq)]
    struct Declared {
        /// The `&str` constants it attributes: `NAME` at column 0, `module::NAME` four columns
        /// into a top-level inline module, and the constant names it re-exports.
        names: Vec<String>,
        /// Every other `const` or `static` line whose type mentions `str`, and every
        /// re-export it cannot read, as `line N: text`. A census fails on any.
        unread: Vec<String>,
    }

    /// `line` without a leading visibility: `pub`, `pub(crate)`, `pub(super)`, `pub(self)` or
    /// `pub(in path)`.
    fn without_visibility(line: &str) -> &str {
        if let Some(rest) = line.strip_prefix("pub(") {
            return rest.split_once(") ").map_or(line, |(_, rest)| rest);
        }
        line.strip_prefix("pub ").unwrap_or(line)
    }

    /// The name of a top-level `mod name {` line (any visibility, any trailing comment),
    /// else `None`. `mod name;` is not inline.
    fn inline_module(line: &str) -> Option<&str> {
        let rest = without_visibility(line).strip_prefix("mod ")?;
        let name = rest.split([' ', '{']).next()?;
        rest.contains('{').then_some(name)
    }

    fn is_const_name(name: &str) -> bool {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
    }

    /// A `const` or `static` item line (after its indentation): its name and its type text up
    /// to `=` (or the line's end). `const fn` is not an item of this kind.
    fn item(line: &str) -> Option<(&str, &str)> {
        let rest = without_visibility(line);
        let rest = rest
            .strip_prefix("const ")
            .or_else(|| rest.strip_prefix("static mut "))
            .or_else(|| rest.strip_prefix("static "))?;
        if rest.starts_with("fn ") {
            return None;
        }
        let (name, ty) = rest.split_once(':')?;
        Some((name.trim(), ty.split('=').next().unwrap_or(ty).trim()))
    }

    /// The constant names a `pub use` line re-exports (`path::NAME;`, `path::X as NAME;`,
    /// `path::{A, B as C};`), or `None` when it is not one line ending `;`.
    fn reexports(line: &str) -> Option<Vec<&str>> {
        let body = line.strip_prefix("pub use ")?.strip_suffix(';')?;
        let items: Vec<&str> = match body.split_once('{') {
            Some((_, group)) => group.strip_suffix('}')?.split(',').collect(),
            None => vec![body],
        };
        Some(
            items
                .into_iter()
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| match item.rsplit_once(" as ") {
                    Some((_, alias)) => alias,
                    None => item.rsplit("::").next().unwrap_or(item),
                })
                .filter(|name| is_const_name(name))
                .collect(),
        )
    }

    /// Read a game module's source. A `tests` module is skipped on purpose (test fixtures are
    /// not sentences); every other `const`/`static` whose type mentions `str` is either
    /// attributed to a name or listed as unread.
    fn declared(source: &str) -> Declared {
        let mut out = Declared::default();
        let mut inner: Option<&str> = None;
        for (index, line) in source.lines().enumerate() {
            let number = index + 1;
            if line == "}" {
                inner = None;
                continue;
            }
            if let Some(name) = inline_module(line) {
                inner = Some(name);
                continue;
            }
            if inner == Some("tests") {
                continue;
            }
            let indent = line.len() - line.trim_start().len();
            let body = line.trim_start();
            let prefix = inner
                .map(|module| format!("{module}::"))
                .unwrap_or_default();
            let depth = if inner.is_some() { 4 } else { 0 };
            let attributable = indent == depth;
            if body.starts_with("pub use ") {
                match reexports(body) {
                    Some(names) if attributable => {
                        out.names
                            .extend(names.into_iter().map(|n| format!("{prefix}{n}")));
                    }
                    Some(names) if names.is_empty() => {}
                    // An indented re-export of a constant, or one this reader cannot parse (a
                    // group across lines).
                    _ => out.unread.push(format!("line {number}: {}", line.trim())),
                }
                continue;
            }
            let Some((name, ty)) = item(body) else {
                continue;
            };
            if !ty.contains("str") {
                continue;
            }
            let plain = ty == "&str" || ty == "&'static str";
            let is_const = !without_visibility(body).starts_with("static");
            if attributable && plain && is_const && is_const_name(name) {
                out.names.push(format!("{prefix}{name}"));
            } else {
                out.unread.push(format!("line {number}: {}", line.trim()));
            }
        }
        out
    }

    /// **Complete**: the census reads every `str` item line of `source`; every constant it
    /// attributes has a row or an exclusion, and every row and exclusion names a constant
    /// still declared there.
    fn census(file: &str, source: &str, rows: &[Row], excluded: &[(&str, &str)]) {
        let declared = declared(source);
        assert!(
            declared.unread.is_empty(),
            "{file} declares string items the bot's census cannot attribute to a row: {:?}. \
             Declare a sentence as a module-level `pub const NAME: &str`, or teach the census.",
            declared.unread
        );
        let known: BTreeSet<&str> = rows
            .iter()
            .map(|row| row.0)
            .chain(excluded.iter().map(|exclusion| exclusion.0))
            .collect();
        let missing: Vec<&str> = declared
            .names
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
            .filter(|name| !declared.names.iter().any(|d| d == name))
            .collect();
        assert!(
            stale.is_empty(),
            "the bot's table has rows for constants {file} no longer declares: {stale:?}"
        );
    }

    /// The census reader itself, on a made-up module holding every shape it must read or
    /// refuse.
    #[test]
    fn the_census_reads_or_refuses_every_shape() {
        let lines = [
            "pub const A: &str = \"a\";",
            "pub(crate) const B_2: &'static str =",
            "    \"b\";",
            "const C: &str = \"c\";",
            "pub(in crate::x) const D: &str = \"d\";",
            "pub const N: u32 = 1;",
            "pub use crate::x::A as E;",
            "pub use crate::x::F;",
            "pub use crate::x::{G, H as I, Thing};",
            "pub use crate::x::Type;",
            "pub use crate::x::{",
            "    J,",
            "};",
            "pub static S: &str = \"s\";",
            "pub const T: &[&str] = &[\"t\"];",
            "pub const U: [&str; 2] = [\"u\", \"v\"];",
            "pub const fn word() -> &'static str {",
            "    \"w\"",
            "}",
            "fn f() {",
            "    const LOCAL: &str = \"l\";",
            "}",
            "impl X {",
            "    pub const IMPLIED: &str = \"i\";",
            "}",
            "    pub const STRAY: &str = \"stray\";",
            "pub mod named { // a comment",
            "    pub const K: &str = \"k\";",
            "    pub mod deeper {",
            "        pub const DEEP: &str = \"d\";",
            "    }",
            "}",
            "/// pub const DOC: &str = \"doc\";",
            "#[cfg(test)]",
            "mod tests {",
            "    pub const FIXTURE: &str = \"f\";",
            "}",
        ];
        let read = declared(&lines.join("\n"));
        assert_eq!(
            read.names,
            ["A", "B_2", "C", "D", "E", "F", "G", "I", "named::K"],
            "{read:?}"
        );
        assert_eq!(
            read.unread,
            [
                "line 11: pub use crate::x::{",
                "line 14: pub static S: &str = \"s\";",
                "line 15: pub const T: &[&str] = &[\"t\"];",
                "line 16: pub const U: [&str; 2] = [\"u\", \"v\"];",
                "line 21: const LOCAL: &str = \"l\";",
                "line 24: pub const IMPLIED: &str = \"i\";",
                "line 26: pub const STRAY: &str = \"stray\";",
                "line 30: pub const DEEP: &str = \"d\";",
            ],
            "{read:?}"
        );
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
        distinct(&qualified("end_user", end_user_rows()), &[]);
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
        distinct(&qualified("renewal", renewal_rows()), &[]);
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
        distinct(&qualified("claim", claim_rows()), &[]);
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
        distinct(&qualified("dispatch", dispatch_rows()), &[]);
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
        distinct(&qualified("absent", absent_rows()), &[]);
    }

    #[test]
    fn ship_mind_needles_are_in_step() {
        in_step("ship_mind", &ship_mind_rows());
    }
    #[test]
    fn ship_mind_census() {
        census(
            "ship_mind.rs",
            SHIP_MIND_RS,
            &ship_mind_rows(),
            SHIP_MIND_EXCLUDED,
        );
    }
    #[test]
    fn ship_mind_needles_are_distinct() {
        distinct(&qualified("ship_mind", ship_mind_rows()), &[]);
    }

    #[test]
    fn practice_needles_are_in_step() {
        in_step("practice", &practice_rows());
    }
    #[test]
    fn practice_census() {
        census("practice.rs", PRACTICE_RS, &practice_rows(), &[]);
    }
    #[test]
    fn practice_needles_are_distinct() {
        distinct(&qualified("practice", practice_rows()), &[]);
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
        distinct(&qualified("market", market_rows()), MARKET_OVERLAPS);
    }

    /// **Distinct across the table**: no needle of any family is found in another family's
    /// sentence, but the listed cross-family pairs.
    #[test]
    fn needles_are_distinct_across_families() {
        let overlaps: Vec<(&str, &str, &str)> = MARKET_OVERLAPS
            .iter()
            .chain(CROSS_OVERLAPS)
            .copied()
            .collect();
        distinct(&all_rows(), &overlaps);
    }

    /// **The walk's needles are the game's words** (doc 44 unit MW.19): each is found, as
    /// written, in the game source that prints it, or in the game's constant where there is
    /// one. A needle that is a whole line is looked for as a whole string literal.
    #[test]
    fn walk_needles_are_in_the_games_own_text() {
        use super::walk;
        const ENGINE_RS: &str = include_str!("../../../mush-core/src/engine.rs");
        const WORLD_TASK_RS: &str = include_str!("../../../mush-server/src/world_task.rs");
        const REGISTRY_RS: &str =
            include_str!("../../../tacsim/src/minds/bulk/registry_service.rs");
        const LOBBY_RS: &str = include_str!("../../../tacsim/src/match_/lobby.rs");
        const NOTICES_RS: &str = include_str!("../../../tacsim/src/match_/cockpit/notices.rs");
        const REPORT_RS: &str = include_str!("../../../tacsim/src/match_/report.rs");
        let whole = |needle: &str| format!("\"{needle}\"");
        let rows: [(&str, &str, String, &str); 14] = [
            ("QUEUED", "engine.rs", whole(walk::QUEUED), ENGINE_RS),
            ("DONE", "world_task.rs", whole(walk::DONE), WORLD_TASK_RS),
            (
                "RATING_QUEUED",
                "registry_service.rs",
                whole(walk::RATING_QUEUED),
                REGISTRY_RS,
            ),
            (
                "RATING_RETAINED",
                "registry_service.rs",
                whole(walk::RATING_RETAINED),
                REGISTRY_RS,
            ),
            (
                "RATING_FINISHED",
                "registry_service.rs",
                whole(walk::RATING_FINISHED),
                REGISTRY_RS,
            ),
            (
                "RATING_REFUSED",
                "registry_service.rs",
                format!("\"{}: ", walk::RATING_REFUSED),
                REGISTRY_RS,
            ),
            (
                "RATING_NOT_ADMITTED",
                "registry_service.rs",
                whole(walk::RATING_NOT_ADMITTED),
                REGISTRY_RS,
            ),
            (
                "SIM_CARRY",
                "lobby.rs",
                format!("\"{}", walk::SIM_CARRY),
                LOBBY_RS,
            ),
            (
                "SIM_SEALED",
                "lobby.rs",
                format!("\"{}", walk::SIM_SEALED),
                LOBBY_RS,
            ),
            (
                "COCKPIT_MIND",
                "notices.rs",
                walk::COCKPIT_MIND.to_owned(),
                NOTICES_RS,
            ),
            (
                "SIM_ON_ITS_OWN",
                "notices.rs",
                whole(walk::SIM_ON_ITS_OWN),
                NOTICES_RS,
            ),
            (
                "SIM_YOUR_MIND",
                "report.rs",
                format!("\"{}", walk::SIM_YOUR_MIND),
                REPORT_RS,
            ),
            (
                "SIM_IT_FLEW",
                "report.rs",
                format!("\"  {}\"", walk::SIM_IT_FLEW),
                REPORT_RS,
            ),
            (
                "REFUSED",
                "action.rs",
                walk::REFUSED.to_owned(),
                mush_core::action::SUBMISSION_REFUSED_LINE,
            ),
        ];
        for (name, file, text, source) in &rows {
            assert!(
                source.contains(text.as_str()),
                "walk::{name}: `{text}` is no longer in the game's {file}"
            );
        }
        assert!(
            tacsim::port::tv::LAUNCHED_DOCTRINE.contains(walk::LAUNCHED_DOCTRINE),
            "walk::LAUNCHED_DOCTRINE"
        );
        assert_eq!(
            mush_core::action::SUBMISSION_REFUSED_LINE,
            walk::REFUSED,
            "walk::REFUSED is the engine's whole line"
        );
        // No needle of the walk is found inside another of its own.
        let needles = [
            walk::QUEUED,
            walk::DONE,
            walk::REFUSED,
            walk::RATING_QUEUED,
            walk::RATING_RETAINED,
            walk::RATING_FINISHED,
            walk::RATING_REFUSED,
            walk::RATING_NOT_ADMITTED,
            walk::SIM_CARRY,
            walk::SIM_SEALED,
            walk::COCKPIT_MIND,
            walk::SIM_ON_ITS_OWN,
            walk::SIM_YOUR_MIND,
            walk::SIM_IT_FLEW,
            walk::LAUNCHED_DOCTRINE,
        ];
        for (i, a) in needles.iter().enumerate() {
            for (j, b) in needles.iter().enumerate() {
                assert!(i == j || !b.contains(a), "`{a}` is inside `{b}`");
            }
        }
    }

    /// **Partial needles are listed**: every row with a short needle is in `PARTIAL`, and
    /// every `PARTIAL` entry is a row.
    #[test]
    fn short_needles_are_listed_as_partial() {
        let rows = all_rows();
        let short: BTreeSet<&str> = rows
            .iter()
            .filter(|row| row.2.trim().chars().count() < super::PARTIAL_BELOW)
            .map(|row| row.0.as_str())
            .collect();
        let listed: BTreeSet<&str> = super::PARTIAL.iter().copied().collect();
        assert_eq!(short, listed, "short needles and `PARTIAL` differ");
    }
}

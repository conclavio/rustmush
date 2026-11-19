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
    pub const CLAIM_REFUSED: &str = "The Registry did not admit the mind's claim; the launch is canceled and the rig stays in its bay.";
    /// `tacsim::surface::excursion::CLAIM_CANCELLED`.
    pub const CLAIM_CANCELLED: &str =
        "The launch could not start; the mind's claim is closed and the rig stays in its bay.";
    /// `tacsim::surface::excursion::CLAIM_NOT_OPEN`.
    pub const CLAIM_NOT_OPEN: &str = "that rig's mind claim has not opened yet";
    /// `tacsim::surface::excursion::CLAIM_ADMITTED` (doc 47 CO.11): its second row's tail.
    pub const CLAIM_ADMITTED: &str = " until the rig is home.";
}

/// A launch's `mind=` refusals (doc 23 S.4b, doc 17h M8e): `tacsim::surface::dispatch`.
pub mod dispatch {
    /// `tacsim::surface::dispatch::MIND_PENDING`.
    pub const MIND_PENDING: &str =
        "is changing hands; it is not sealed aboard until the transfer settles.";
    /// `tacsim::surface::dispatch::MIND_NO_LICENCE`.
    pub const MIND_NO_LICENCE: &str = "You hold no license";
    /// `tacsim::surface::dispatch::MIND_GOVERNED_DRAFT`.
    pub const MIND_GOVERNED_DRAFT: &str =
        "A governed core seals no unfiled draft; name a filed license.";
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
    pub const MIND_NOT_FOR_ORG: &str = "You do not act for that organization.";
}

/// A field unit's mind taking it home while its pilot is away (doc 44 MW.7):
/// `tacsim::field::absent`.
pub mod absent {
    /// `tacsim::field::absent::MIND_HAS_IT`.
    pub const MIND_HAS_IT: &str = "its mind has the controls and is taking it home. It fires only on a unit that fires on it.";
    /// `tacsim::field::absent::TAKEN_HOME`.
    pub const TAKEN_HOME: &str = "passed to its mind, which makes for its carrier's ramp.";
    /// `tacsim::field::absent::BACK_IN_SEAT`.
    pub const BACK_IN_SEAT: &str = "its mind stands down and what it had pending is dropped.";
    /// `tacsim::field::absent::WENT_HOME`.
    pub const WENT_HOME: &str = "out of the field at its carrier's ramp.";
    /// `tacsim::field::absent::LEFT_AT_RAMP`.
    pub const LEFT_AT_RAMP: &str = "out of the field, but it could not be put aboard.";
    /// `tacsim::field::absent::PUT_ABOARD` (doc 47 CO.11b).
    pub const PUT_ABOARD: &str =
        "out of the field at its carrier's ramp, and it is aboard her now.";
    /// `tacsim::field::absent::ADMITTED_AWAY` (doc 45 cut AP.5).
    pub const ADMITTED_AWAY: &str =
        "while you were away: its mind has the controls and makes for its carrier's ramp.";
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
        "with nobody in its seat: its mind has the controls and makes for its carrier's ramp.";
    /// `tacsim::field::absent::UNATTENDED_ON` (doc 45 unit AP.7).
    pub const UNATTENDED_ON: &str =
        "with nobody in its seat: its mind has the controls and flies it on ";
}

/// `+ship/mind`, a mind sealed aboard a ship (doc 44 MW.13): `tacsim::port::ship_mind`.
pub mod ship_mind {
    /// `tacsim::port::ship_mind::MIND_USAGE`.
    pub const MIND_USAGE: &str = "Usage: +ship/mind | +ship/mind filed:<license>";
    /// `tacsim::port::ship_mind::MIND_NONE`.
    pub const MIND_NONE: &str = "If you lose your link, the house default takes her home.";
    /// `tacsim::port::ship_mind::MIND_ADMITTING`.
    pub const MIND_ADMITTING: &str = "The Registry is admitting license ";
    /// `tacsim::port::ship_mind::MIND_ABOARD`.
    pub const MIND_ABOARD: &str =
        "It has her helm and sensors while her crew has no link, until she next docks at a port.";
    /// `tacsim::port::ship_mind::MIND_UNFIT`.
    pub const MIND_UNFIT: &str = "It no longer fits her; its claim is being closed.";
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
        "and holds a seat of the license until she next docks at a port. Add go to seal it.";
    /// `tacsim::port::ship_mind::MIND_NOT_SEALED`.
    pub const MIND_NOT_SEALED: &str = " has no mind sealed aboard to unseal.";
    /// `tacsim::port::ship_mind::MIND_NOT_YOURS`.
    pub const MIND_NOT_YOURS: &str = " was sealed by someone else; they unseal it.";
    /// `tacsim::port::ship_mind::MIND_NOT_YET`.
    pub const MIND_NOT_YET: &str = "'s mind claim yet; try again in a moment.";
    /// `tacsim::port::ship_mind::MIND_OFF_PREVIEW`.
    pub const MIND_OFF_PREVIEW: &str = ": its claim closes and the license's seat is free.";
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
        " is held as she is while a mind is sealed aboard: +ship/mind off, at her berth.";
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
    pub const ATTESTING: &str = "Renewing your owner's license for filed mind";
    /// `tacsim::minds::words::FORKING`.
    pub const FORKING: &str = "Forking license";
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
    pub const TERMS_LINE: &str = "days of Grace; the license";
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
    pub const HOLD_HEAD: &str = "Your mind licenses:";
    /// `tacsim::minds::words::HOLD_OWNER`. Partial: a holdings row's indent; a reader takes it
    /// only from a line that starts with it, after `HOLD_HEAD`, spaces not collapsed.
    pub const HOLD_OWNER: &str = "  filed ";
    /// `tacsim::minds::words::HOLD_MORE`.
    pub const HOLD_MORE: &str = "More: +minds/holdings";
    /// `tacsim::minds::words::HOLD_EMPTY`.
    pub const HOLD_EMPTY: &str = "You hold no mind licenses.";
    /// `tacsim::minds::words::YOUR_QUOTE`.
    pub const YOUR_QUOTE: &str = "more seconds. Pay it with +minds/buy/pay";
    /// `tacsim::minds::words::QUOTE_EXPIRED`.
    pub const QUOTE_EXPIRED: &str = "Your quote has expired. Ask for a new one with +minds/buy";
    /// `tacsim::minds::words::YOUR_ESCROW`.
    pub const YOUR_ESCROW: &str =
        "Your payment is held in escrow. Take the license with +minds/buy/complete";
    /// `tacsim::minds::words::ASK`.
    pub const ASK: &str = "Ask for a quote with +minds/buy";
    /// `tacsim::minds::words::NO_QUOTE`.
    pub const NO_QUOTE: &str = "Ask for one with +minds/buy";
    /// `tacsim::minds::words::PAYING`.
    pub const PAYING: &str = "Paying your quote on listing";
    /// `tacsim::minds::words::NO_PURCHASE`.
    pub const NO_PURCHASE: &str = "You have no paid purchase waiting on listing";
    /// `tacsim::minds::words::COMPLETING`.
    pub const COMPLETING: &str = "Taking the license from listing";
    /// `tacsim::minds::words::REFUNDING`.
    pub const REFUNDING: &str = "Asking for your payment on listing";
    /// `tacsim::minds::words::HOLD_PAID`. Partial: a holdings row's indent; a reader takes it
    /// only from a line that starts with it, after `HOLD_HEAD`, spaces not collapsed.
    pub const HOLD_PAID: &str = "  license ";
    /// `tacsim::minds::words::HOLD_WAIT_HEAD`.
    pub const HOLD_WAIT_HEAD: &str = "Purchases waiting:";
    /// `tacsim::minds::words::HOLD_WAIT`.
    pub const HOLD_WAIT: &str = "paid into escrow: +minds/buy/complete";
    /// `tacsim::minds::words::NO_LICENCE`.
    pub const NO_LICENCE: &str = "You hold no license";
    /// `tacsim::minds::words::NOT_FOR_ORG`.
    pub const NOT_FOR_ORG: &str = "You do not act for that organization.";
    /// `tacsim::minds::words::NO_RENEWAL`.
    pub const NO_RENEWAL: &str =
        "has no renewal offer waiting for you. Ask for one with +minds/renew";
    /// `tacsim::minds::words::RESELLING`.
    pub const RESELLING: &str = "Offering license";
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
    pub const REVOKING: &str = "Revoking license";
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
    pub const PRINCIPALS_BARRED: &str = "The Registry has barred your mind licenses until";
    /// `tacsim::minds::words::PRINCIPAL_REVOKED`.
    pub const PRINCIPAL_REVOKED: &str = "holds no new license until";
    /// `tacsim::minds::words::NO_SUCH_PRINCIPAL`.
    pub const NO_SUCH_PRINCIPAL: &str = "No such character or organization.";
    /// `tacsim::minds::words::AMBIGUOUS_PRINCIPAL`.
    pub const AMBIGUOUS_PRINCIPAL: &str =
        "That name is a character and an organization; name the character as #<n>.";
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
        pub const NO_OFFER: &str = "You have no offer for license";
        /// `tacsim::minds::words::named::NO_OFFER_AT`.
        pub const NO_OFFER_AT: &str = "None of your offers for license";
        /// `tacsim::minds::words::named::CHARTER_OFFER`.
        pub const CHARTER_OFFER: &str = "and its charter are offered to you for";
        /// `tacsim::minds::words::named::CHARTER_ACCEPT_HOW`.
        pub const CHARTER_ACCEPT_HOW: &str = "Accept it with +minds/charter/accept";
        /// `tacsim::minds::words::named::NO_CHARTER_OFFER`.
        pub const NO_CHARTER_OFFER: &str = "You have no offer for filed mind";
        /// `tacsim::minds::words::named::NO_CHARTER_OFFER_AT`.
        pub const NO_CHARTER_OFFER_AT: &str = "None of your offers for filed mind";
        /// `tacsim::minds::words::named::ACCEPTING`.
        pub const ACCEPTING: &str = "Accepting the offer for license";
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
    /// `engine.rs`. A whole line; a reader takes a line that is exactly it. Only for a line
    /// the player typed (doc 47 CO.14): a step the engine takes on the player's behalf, a
    /// Registry rating's among them, prints neither this nor [`DONE`].
    pub const QUEUED: &str = "Queued.";
    /// The world task's answer when that commit landed and its provider has no sentence of
    /// its own: `mush-server`, `world_task.rs`. A whole line; a reader takes a line that is
    /// exactly it.
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

/// **The exchange's personal seller** (doc 48 cut EX.1a): `+market/post`, `+market/bid`,
/// `+market/withdraw` and `+market/mine`, and the refusals a player hears from them and from a
/// private offer: `mush_market::seller`. No lane reads them yet; each is held to the game's own
/// text below, so a lane that does starts from a needle that matches.
pub mod exchange {
    /// `mush_market::seller::ASK_POSTED`'s head: an exact lot listed.
    pub const ASK_POSTED: &str = "Listed for sale as #";
    /// `mush_market::seller::POSTED_AWAY`: the listing or bid trades while its poster is away.
    pub const POSTED_AWAY: &str = "It trades while you are away.";
    /// `mush_market::seller::BID_POSTED`'s tail: a bid posted and escrowed.
    pub const BID_POSTED: &str = " is held from your account.";
    /// `mush_market::seller::FOR_ONLY`: a private offer's clause on a posted line.
    pub const FOR_ONLY: &str = " (a private offer to ";
    /// `mush_market::seller::PRIVATE_MARK`: a listing line's mark on a private offer.
    pub const PRIVATE_MARK: &str = "(private offer)";
    /// `mush_market::seller::BID_SUPPLY`: `+market/show`'s supply line for a player's bid.
    pub const BID_SUPPLY: &str = "the bidder buys this";
    /// `mush_market::seller::WITHDRAWN`'s tail: a listing withdrawn.
    pub const WITHDRAWN: &str = "its goods are yours to use again.";
    /// `mush_market::seller::BID_WITHDRAWN`'s tail: a bid withdrawn, its escrow released.
    pub const BID_WITHDRAWN: &str = "what it still held is yours again.";
    /// `mush_market::seller::MINE_HEAD`: `+market/mine`'s heading.
    pub const MINE_HEAD: &str = "Your listings:";
    /// `mush_market::seller::MINE_NONE`.
    pub const MINE_NONE: &str = "You have nothing listed.";
    /// `mush_market::seller::MINE_COUNT`'s tail.
    pub const MINE_COUNT: &str = " listings a seller may.";
    /// `mush_market::seller::NO_SUCH_LOT`.
    pub const NO_SUCH_LOT: &str = "No such lot.";
    /// `mush_market::seller::NO_SUCH_VAULT`.
    pub const NO_SUCH_VAULT: &str = "No such vault.";
    /// `mush_market::seller::LOT_HELD`.
    pub const LOT_HELD: &str = "That lot is held for something else";
    /// `mush_market::seller::NOT_A_LOT_GOOD`.
    pub const NOT_A_LOT_GOOD: &str = "This market does not list that good";
    /// `mush_market::seller::LOT_TOO_BIG`.
    pub const LOT_TOO_BIG: &str = "larger than one listing sells";
    /// `mush_market::seller::SELLER_FULL`: the 32-listing cap.
    pub const SELLER_FULL: &str = "as many as a seller may. Withdraw one first.";
    /// `mush_market::seller::NAME_VENUE`.
    pub const NAME_VENUE: &str = "Name the venue it trades in";
    /// `mush_market::seller::NO_CURRENCY`.
    pub const NO_CURRENCY: &str = "trades in no currency yet";
    /// `mush_market::seller::NO_ACCOUNT`.
    pub const NO_ACCOUNT: &str = "You have no account in that venue's currency";
    /// `mush_market::seller::PRIVATE_NAMES_OTHER`.
    pub const PRIVATE_NAMES_OTHER: &str = "A private offer names one other player";
    /// `mush_market::seller::PRIVATE_TO_OTHER`: a private offer refused to anyone but its
    /// counterparty, at quote and at accept.
    pub const PRIVATE_TO_OTHER: &str = "That offer is made to someone else";
    /// `mush_market::seller::OWN_LISTING`'s head.
    pub const OWN_LISTING: &str = "That is your own listing";
    /// `mush_market::seller::BID_TWICE`'s head.
    pub const BID_TWICE: &str = "You already bid for that good";
    /// `mush_market::seller::BID_INTO_HOLD`'s head.
    pub const BID_INTO_HOLD: &str = "A bid for bulk goods receives into a vault, not a hold";
    /// `mush_market::seller::BID_CANNOT_COVER`'s tail.
    pub const BID_CANNOT_COVER: &str = " available to hold for that bid";
    /// `mush_market::seller::BID_NEEDS_APPROVAL`'s head.
    pub const BID_NEEDS_APPROVAL: &str = "Holding that bid needs an approval";
    /// `mush_market::seller::BID_NOT_HELD`.
    pub const BID_NOT_HELD: &str = "That bid cannot be held right now";
    /// `mush_market::seller::BIDDER_NOT_BUYING`.
    pub const BIDDER_NOT_BUYING: &str = "That bid is not buying any more";
    /// `mush_market::seller::SELLER_GONE`.
    pub const SELLER_GONE: &str = "That seller cannot complete the sale right now";
    /// `mush_market::seller::POST_USAGE`'s head.
    pub const POST_USAGE: &str = "Usage: +market/post ";
    /// `mush_market::seller::BID_USAGE`'s head.
    pub const BID_USAGE: &str = "Usage: +market/bid ";
    /// `mush_market::seller::WITHDRAW_USAGE`.
    pub const WITHDRAW_USAGE: &str = "Usage: +market/withdraw <listing>";
    /// `mush_market::seller::NO_SUCH_PLAYER`.
    pub const NO_SUCH_PLAYER: &str = "No such player.";
    /// `mush_market::seller::ASSET_POSTED`'s middle (doc 48 cut EX.2): a hull listed.
    pub const ASSET_POSTED: &str = " up for sale as #";
    /// `mush_market::seller::NO_SUCH_ASSET` (doc 48 cut EX.2).
    pub const NO_SUCH_ASSET: &str = "No such asset.";
    /// `mush_market::seller::NOT_LISTABLE` (doc 48 cut EX.2).
    pub const NOT_LISTABLE: &str = "That is not something this market lists for you.";
    /// `mush_market::seller::NOT_FOR_ORG` (doc 48 cut PL.9): `as=<org>` for an org not yours.
    pub const NOT_FOR_ORG: &str = "You do not act for that organization, so nothing was listed.";
    /// `mush_market::seller::ALREADY_FOR_SALE`'s head (doc 48 cut EX.2).
    pub const ALREADY_FOR_SALE: &str = "That is already up for sale";
}

/// **The job board** (doc 48 cut EX.5): `+market/jobs` (`mush_market::services`) and the
/// retrieval join's new sentences (`tacsim::surface::retrieval`). No lane reads them yet; each is
/// held to the game's own text below.
pub mod jobs {
    /// `mush_market::services::JOBS_USAGE`.
    pub const JOBS_USAGE: &str = "Usage: +market/jobs [retrieval|passage|survey|crew|haul|escort]";
    /// `mush_market::services::JOBS_NO_VENUE`.
    pub const JOBS_NO_VENUE: &str = "You are not at a market here; name one with market=";
    /// `mush_market::services::JOBS_NONE`.
    pub const JOBS_NONE: &str = "No jobs are posted on this board.";
    /// `mush_market::services::JOBS_HEAD`'s head.
    pub const JOBS_HEAD: &str = "Jobs on this board (";
    /// `mush_market::services::JOB_LINE`'s middle: a job's pay. Partial: read the amount after it.
    pub const JOB_LINE: &str = ". Pays ";
    /// `mush_market::services::JOB_NEGOTIABLE`: a pay that is an opening figure. Partial: read
    /// it after the amount on a job's line.
    pub const JOB_NEGOTIABLE: &str = " to open";
    /// `mush_market::services::JOB_BOND`'s head. Partial: read it on a job's line.
    pub const JOB_BOND: &str = ", bond ";
    /// `mush_market::services::JOB_TAKE`'s tail: the verb that takes the job. Partial: read the
    /// command after it.
    pub const JOB_TAKE: &str = ". Take it: ";
    /// `mush_market::services::TERM_FROM_ACCEPTANCE`'s tail.
    pub const TERM_FROM_ACCEPTANCE: &str = " IC days from acceptance";
    /// `mush_market::services::TERM_BY`'s head. Partial: read the days after it.
    pub const TERM_BY: &str = "due within ";
    /// `mush_market::services::TERM_TODAY`. Partial: read it on a job's line.
    pub const TERM_TODAY: &str = "due today";
    /// `mush_market::services::TERM_ON_PERFORMANCE`.
    pub const TERM_ON_PERFORMANCE: &str = "paid on arrival";
    /// `mush_market::services::TERM_DAILY` (doc 48 cut EX.7).
    pub const TERM_DAILY: &str = "paid by the IC day";
    /// `mush_market::services::JOBS_MORE`.
    pub const JOBS_MORE: &str = "(More are posted than one board shows.)";
    /// `tacsim::surface::retrieval::ACCEPT_USAGE`'s head.
    pub const ACCEPT_USAGE: &str = "Usage: +salvage/accept <job> [as=<org>] [go]";
    /// `tacsim::surface::retrieval::BOND_CLAUSE`'s head: the bond a job's taker escrows.
    pub const BOND_CLAUSE: &str = " You escrow a bond of ";
    /// `tacsim::surface::retrieval::NOT_FOR_ORG`.
    pub const NOT_FOR_ORG: &str = "You do not act for that organization, so nothing was accepted.";
    /// `tacsim::surface::retrieval::ORG_NO_ACCOUNT`.
    pub const ORG_NO_ACCOUNT: &str = "That organization has no account to be paid into.";
}

/// **Crew wages and crew for hire** (doc 48 cut EX.7): `+sortie/crew`'s wage, a contract's wage
/// line and `+sortie/available` (`tacsim::detachment::crew::wages`). The DT lane reads
/// [`crew::OFFERS_WAGES`] as the offers' end; the rest are held to the game's own text below.
pub mod crew {
    /// `tacsim::detachment::crew::wages::OFFER_WAGE`'s head. Partial: read the wage after it.
    pub const OFFER_WAGE: &str = "; wage ";
    /// `tacsim::detachment::crew::wages::OFFERS_WAGES`'s head: the offers' footer.
    pub const OFFERS_WAGES: &str = "A hire's wage is ";
    /// `tacsim::detachment::crew::wages::OFFERS_ABOARD`.
    pub const OFFERS_ABOARD: &str = "Hiring does not move the character aboard.";
    /// `tacsim::detachment::crew::wages::WAGE_LINE`'s head. Partial: read the wage after it.
    pub const WAGE_LINE: &str = "Wage: ";
    /// `tacsim::detachment::crew::wages::WAGE_NONE`.
    pub const WAGE_NONE: &str =
        "No wage is charged: the hiring port has no authority to pay it to.";
    /// `tacsim::detachment::crew::wages::LIST_USAGE`'s head.
    pub const LIST_USAGE: &str = "Usage: +sortie/available <duty>[,<duty>] [officer]";
    /// `tacsim::detachment::crew::wages::LISTED`'s tail. Partial: read the wage before it.
    pub const LISTED: &str = " an IC day.";
    /// `tacsim::detachment::crew::wages::LISTING_SHOW`'s tail.
    pub const LISTING_SHOW: &str = ", shown there on its board.";
    /// `tacsim::detachment::crew::wages::NOT_LISTED`.
    pub const NOT_LISTED: &str = "You are not listed for crew hire.";
    /// `tacsim::detachment::crew::wages::UNLISTED`.
    pub const UNLISTED: &str = "You are no longer listed for crew hire.";
    /// `tacsim::detachment::crew::wages::LIST_AT_PORT`.
    pub const LIST_AT_PORT: &str = "List yourself for crew hire from a port's station.";
    /// `tacsim::detachment::crew::wages::BOARD_SUBJECT`'s middle: a listed player on the board.
    pub const BOARD_SUBJECT: &str = " for hire (";
    /// `tacsim::detachment::crew::wages::BOARD_TAKE`'s head. Partial: read the name after it.
    pub const BOARD_TAKE: &str = "page ";
}

/// **A discharge at a port** (doc 48 cut EX.7b): `+sortie/discharge`
/// (`tacsim::detachment::crew::discharge`). No lane reads them yet; each is held to the game's
/// own text below.
pub mod discharge {
    /// `tacsim::detachment::crew::discharge::DISCHARGE_USAGE`'s head.
    pub const DISCHARGE_USAGE: &str = "Usage: +sortie/discharge #<npc>";
    /// `tacsim::detachment::crew::discharge::NOT_YOURS`.
    pub const NOT_YOURS: &str =
        "No crew contract of yours names that character: only the employer discharges a hire.";
    /// `tacsim::detachment::crew::discharge::AT_PORT`.
    pub const AT_PORT: &str =
        "Discharge a hire from a port's station: its hall settles the final wage.";
    /// `tacsim::detachment::crew::discharge::ON_SORTIE`'s tail. Partial: the name before it.
    pub const ON_SORTIE: &str = " is out on a sortie: ";
    /// `tacsim::detachment::crew::discharge::IN_FIELD`'s tail. Partial: the name before it.
    pub const IN_FIELD: &str = " is in an open field: ";
    /// `tacsim::detachment::crew::discharge::CAPTIVE`'s tail. Partial: the name before it.
    pub const CAPTIVE: &str = " is in custody: ";
    /// `tacsim::detachment::crew::discharge::BUSY`.
    pub const BUSY: &str =
        "One of your requests is still being settled: discharge again in a moment.";
    /// `tacsim::detachment::crew::discharge::NOT_NOW`.
    pub const NOT_NOW: &str = "The hiring hall cannot settle a discharge just now";
    /// `tacsim::detachment::crew::discharge::DISCHARGED_PAID`'s tail. Partial: the payee before.
    pub const DISCHARGED_PAID: &str = " as the final wage.";
    /// `tacsim::detachment::crew::discharge::DISCHARGED_OWING`'s middle. Partial: the debt after.
    pub const DISCHARGED_OWING: &str = " is discharged: the final wage, ";
    /// `tacsim::detachment::crew::discharge::DISCHARGED_PART`'s middle. Partial: the debt after.
    pub const DISCHARGED_PART: &str = " of the final wage is paid and ";
    /// `tacsim::detachment::crew::discharge::DISCHARGED_FREE`'s tail.
    pub const DISCHARGED_FREE: &str = " is discharged: no wage was charged, so nothing is owed.";
    /// `tacsim::detachment::crew::discharge::RETURNED`'s head. Partial: the name and room after.
    pub const RETURNED: &str = "The contract is ended. ";
    /// `tacsim::detachment::crew::discharge::STANDING_DEBT`'s tail.
    pub const STANDING_DEBT: &str =
        " of its wage is your debt, collected when your account covers it.";
}

/// **The price history** (doc 48 cut EX.8): `+market/history` (`mush_market::history`). No lane
/// reads them yet; each is held to the game's own text below.
pub mod history {
    /// `mush_market::history::HISTORY_USAGE`.
    pub const HISTORY_USAGE: &str = "Usage: +market/history <catalog>:<code>";
    /// `mush_market::history::HISTORY_NO_VENUE`'s head.
    pub const HISTORY_NO_VENUE: &str = "Name a market with market=";
    /// `mush_market::history::HISTORY_NONE`'s head.
    pub const HISTORY_NONE: &str = "No price history is kept here for ";
    /// `mush_market::history::HISTORY_HEAD`'s head.
    pub const HISTORY_HEAD: &str = "House prices of ";
    /// `mush_market::history::HISTORY_ROW`'s middle. Partial: read the price after it.
    pub const HISTORY_ROW: &str = ": ask ";
    /// `mush_market::history::HISTORY_THIS_WEEK`.
    pub const HISTORY_THIS_WEEK: &str = "this IC week";
    /// `mush_market::history::HISTORY_WEEKS_AGO`'s tail.
    pub const HISTORY_WEEKS_AGO: &str = " IC weeks ago";
    /// `mush_market::history::HISTORY_LAST_WEEK`.
    pub const HISTORY_LAST_WEEK: &str = "last IC week";
    /// `mush_market::history::HISTORY_NO_PRICE`. Partial: read it after "ask " or "bid ".
    pub const HISTORY_NO_PRICE: &str = "none";
}

/// **A hull listed on the exchange** (doc 48 cut EX.2): what `+market/post #<hull>` refuses in
/// the game's words, `tacsim::port::hull_sale`. No lane reads them yet; each is held to the
/// game's own text below.
pub mod hull_sale {
    /// `tacsim::port::hull_sale::HULL_CORED`: a governed core fitted; since doc 48 unit EX.2b
    /// she sells with it by private offer.
    pub const HULL_CORED: &str = "sells only with her governed core, by private offer";
    /// `tacsim::port::hull_sale::HULL_HOLD_LADEN`.
    pub const HULL_HOLD_LADEN: &str = "hold must be empty before she is listed";
    /// `tacsim::port::hull_sale::HULL_HOLD_DARK`'s tail.
    pub const HULL_HOLD_DARK: &str = "hold cannot be read just now; nothing was listed.";
    /// `tacsim::port::hull_sale::HULL_IN_BAY`'s tail.
    pub const HULL_IN_BAY: &str = "stow her before she is listed.";
    /// `tacsim::port::hull_sale::HULL_RIG_ABOARD`'s tail.
    pub const HULL_RIG_ABOARD: &str = "return it to Occator before she is listed.";
}

/// **A governed core resold to a player** (doc 48 unit EX.2b): `+core/offer`, `+core/offers` and
/// `+core/accept`, in the game's words, `tacsim::hardware::sale::resale`. No lane reads them
/// yet; each is held to the game's own text below.
/// **Doc 48 cut EX.6**: the haul's sentences (`tacsim::services::haul`).
pub mod haul {
    /// `tacsim::services::haul::HAUL_USAGE`.
    pub const HAUL_USAGE: &str = "Usage: +haul  |  +haul/post <qty> <good> to=<port> fee=<cr>";
    /// `tacsim::services::haul::POST_PREVIEW`.
    pub const POST_PREVIEW: &str = "). +haul/post ... go posts it.";
    /// `tacsim::services::haul::POSTED`.
    pub const POSTED: &str = "escrowed and paid on delivery.";
    /// `tacsim::services::haul::NO_VENUE`.
    pub const NO_VENUE: &str = "There is no market here to ship from.";
    /// `tacsim::services::haul::NO_LOCKER`.
    pub const NO_LOCKER: &str = "A haul ships from your locker, and you have none here yet (+";
    /// `tacsim::services::haul::SHORT`.
    pub const SHORT: &str = "Your locker here holds too little";
    /// `tacsim::services::haul::NO_GOOD`.
    pub const NO_GOOD: &str = "No good answers to";
    /// `tacsim::services::haul::NO_PORT`.
    pub const NO_PORT: &str = "; name a port of the lane.";
    /// `tacsim::services::haul::SAME_PORT`.
    pub const SAME_PORT: &str = "A haul goes to another port.";
    /// `tacsim::services::haul::OVER_CAP`.
    pub const OVER_CAP: &str = "A declared value is at most the reference price of the goods";
    /// `tacsim::services::haul::NO_REFERENCE`.
    pub const NO_REFERENCE: &str = "That good has no reference price, so its value cannot be dec";
    /// `tacsim::services::haul::POST_FAILED`.
    pub const POST_FAILED: &str = "The haul could not be posted:";
    /// `tacsim::services::haul::NO_SUCH_JOB`.
    pub const NO_SUCH_JOB: &str = "; +market/jobs haul lists them.";
    /// `tacsim::services::haul::OWN_JOB`.
    pub const OWN_JOB: &str = "That haul is your own.";
    /// `tacsim::services::haul::NOT_DOCKED`.
    pub const NOT_DOCKED: &str = ", and your hull is not docked there.";
    /// `tacsim::services::haul::NO_ROOM`.
    pub const NO_ROOM: &str = "Your hold has no room for those goods.";
    /// `tacsim::services::haul::GOODS_GONE`.
    pub const GOODS_GONE: &str = "The shipper's goods are no longer there to load.";
    /// `tacsim::services::haul::ACCEPT_PREVIEW`.
    pub const ACCEPT_PREVIEW: &str = ". The goods are declared at";
    /// `tacsim::services::haul::ACCEPTED`.
    pub const ACCEPTED: &str = ". The goods load into your hold, held for the job; deliver t";
    /// `tacsim::services::haul::ACCEPT_FAILED`.
    pub const ACCEPT_FAILED: &str = "The haul could not be taken:";
    /// `tacsim::services::haul::TOLD_TAKEN`.
    pub const TOLD_TAKEN: &str = "has taken haul";
    /// `tacsim::services::haul::NOT_THERE`.
    pub const NOT_THERE: &str = "Your hull is not docked at";
    /// `tacsim::services::haul::TOO_LATE`.
    pub const TOO_LATE: &str = "The haul's term has ended; the goods go back to the shipper.";
    /// `tacsim::services::haul::NOT_ABOARD`.
    pub const NOT_ABOARD: &str = "You carry no goods for haul";
    /// `tacsim::services::haul::DELIVER_PREVIEW`.
    pub const DELIVER_PREVIEW: &str = ". +haul/deliver <job> go hands them over.";
    /// `tacsim::services::haul::DELIVERING`.
    pub const DELIVERING: &str = "are handed over; the fee and your bond follow.";
    /// `tacsim::services::haul::TOLD_DELIVERED`.
    pub const TOLD_DELIVERED: &str = "is delivered to your locker at";
    /// `tacsim::services::haul::TOLD_RETURNED`.
    pub const TOLD_RETURNED: &str = "was not delivered in time; its goods are back in your locker";
    /// `tacsim::services::haul::NOT_WITHDRAWABLE`.
    pub const NOT_WITHDRAWABLE: &str =
        "Only the shipper withdraws a haul, and only before it is tak";
    /// `tacsim::services::haul::WITHDRAWN`.
    pub const WITHDRAWN: &str = "is withdrawn; the fee comes back.";
    /// `tacsim::services::haul::NONE`.
    pub const NONE: &str = "You have no hauls.";
    /// `tacsim::services::haul::HEAD`.
    pub const HEAD: &str = "Your hauls, posted and taken:";
    /// `tacsim::services::haul::ROW`.
    pub const ROW: &str = "on hire from";
}

/// **Doc 48 cut EX.6**: the escort's sentences (`tacsim::services::escort`).
pub mod escort {
    /// `tacsim::services::escort::ESCORT_USAGE`.
    pub const ESCORT_USAGE: &str = "Usage: +escort  |  +escort/post <hull> to=<port>|field=<site";
    /// `tacsim::services::escort::POST_PREVIEW`.
    pub const POST_PREVIEW: &str = "IC days. +escort/post ... go posts it.";
    /// `tacsim::services::escort::POSTED`.
    pub const POSTED: &str = "escrowed and paid when she is seen through.";
    /// `tacsim::services::escort::NOT_YOURS`.
    pub const NOT_YOURS: &str = "You hold no title to that hull and do not fly her.";
    /// `tacsim::services::escort::NOT_DOCKED`.
    pub const NOT_DOCKED: &str = "She is not docked at a port, so there is no passage to escor";
    /// `tacsim::services::escort::NO_PORT`.
    pub const NO_PORT: &str = "; she has no passage there.";
    /// `tacsim::services::escort::SAME_PORT`.
    pub const SAME_PORT: &str = "Her passage goes to another port.";
    /// `tacsim::services::escort::NO_VENUE`.
    pub const NO_VENUE: &str = "There is no market here to post at.";
    /// `tacsim::services::escort::ALREADY`.
    pub const ALREADY: &str = "An escort of her is open already (";
    /// `tacsim::services::escort::POST_FAILED`.
    pub const POST_FAILED: &str = "The escort could not be posted:";
    /// `tacsim::services::escort::NO_SUCH_JOB`.
    pub const NO_SUCH_JOB: &str = "; +market/jobs escort lists them.";
    /// `tacsim::services::escort::OWN_JOB`.
    pub const OWN_JOB: &str = "That escort is your own.";
    /// `tacsim::services::escort::SAME_HULL`.
    pub const SAME_HULL: &str = "She cannot escort herself.";
    /// `tacsim::services::escort::ACCEPT_PREVIEW`.
    pub const ACCEPT_PREVIEW: &str = "Take the escort: stay near";
    /// `tacsim::services::escort::BOND_CLAUSE`.
    pub const BOND_CLAUSE: &str = "is escrowed: it comes back with the fee if she is seen throu";
    /// `tacsim::services::escort::ACCEPTED`.
    pub const ACCEPTED: &str = "for the whole journey.";
    /// `tacsim::services::escort::ACCEPT_FAILED`.
    pub const ACCEPT_FAILED: &str = "The escort could not be taken:";
    /// `tacsim::services::escort::TOLD_TAKEN`.
    pub const TOLD_TAKEN: &str = "has taken escort";
    /// `tacsim::services::escort::TOLD_STARTED`.
    pub const TOLD_STARTED: &str = ": the journey has begun.";
    /// `tacsim::services::escort::TOLD_KEPT`.
    pub const TOLD_KEPT: &str = "of the journey. The fee follows.";
    /// `tacsim::services::escort::TOLD_FAILED`.
    pub const TOLD_FAILED: &str = "has failed, and its fee is not paid:";
    /// `tacsim::services::escort::WHY_WRECKED`.
    pub const WHY_WRECKED: &str = "she was wrecked";
    /// `tacsim::services::escort::WHY_CAPTURED`.
    pub const WHY_CAPTURED: &str = "she was captured";
    /// `tacsim::services::escort::WHY_LEFT`.
    pub const WHY_LEFT: &str = "the escort left the journey";
    /// `tacsim::services::escort::WHY_FAR`.
    pub const WHY_FAR: &str = "the escort was near her for only";
    /// `tacsim::services::escort::NOT_WITHDRAWABLE`.
    pub const NOT_WITHDRAWABLE: &str =
        "Only the poster withdraws an escort, and only before it is t";
    /// `tacsim::services::escort::WITHDRAWN`.
    pub const WITHDRAWN: &str = "is withdrawn; your fee is back in your account.";
    /// `tacsim::services::escort::NONE`.
    pub const NONE: &str = "You have no escorts.";
    /// `tacsim::services::escort::HEAD`.
    pub const HEAD: &str = "Your escorts:";
    /// `tacsim::services::escort::ROW`.
    pub const ROW: &str = ", escorted on";
}

pub mod core_resale {
    /// `tacsim::hardware::sale::resale::RESALE_OFFER_USAGE`.
    pub const RESALE_OFFER_USAGE: &str = "Usage: +core/offer <core or ship>=<player> price=<n>";
    /// `tacsim::hardware::sale::resale::RESALE_ACCEPT_USAGE`.
    pub const RESALE_ACCEPT_USAGE: &str = "Usage: +core/accept <core or ship>";
    /// `tacsim::hardware::sale::resale::RESALE_OFF`.
    pub const RESALE_OFF: &str = "do not change hands between players";
    /// `tacsim::hardware::sale::resale::RESALE_NOT_CORE`.
    pub const RESALE_NOT_CORE: &str = "neither a governed core nor a ship of yours";
    /// `tacsim::hardware::sale::resale::RESALE_NOT_YOURS`.
    pub const RESALE_NOT_YOURS: &str = "title is not yours alone, so you cannot offer it";
    /// `tacsim::hardware::sale::resale::RESALE_NOT_HELD`.
    pub const RESALE_NOT_HELD: &str = "You are not holding that core.";
    /// `tacsim::hardware::sale::resale::RESALE_FITTED`'s middle.
    pub const RESALE_FITTED: &str = "offer her with it: +core/offer #";
    /// `tacsim::hardware::sale::resale::RESALE_DESTROYED`.
    pub const RESALE_DESTROYED: &str = "A destroyed core is not offered for sale.";
    /// `tacsim::hardware::sale::resale::RESALE_NO_PLAYER`.
    pub const RESALE_NO_PLAYER: &str = "There is no player by that name to offer it to.";
    /// `tacsim::hardware::sale::resale::RESALE_SELF`.
    pub const RESALE_SELF: &str = "You cannot sell a core to yourself.";
    /// `tacsim::hardware::sale::resale::RESALE_PRICE`'s tail.
    pub const RESALE_PRICE: &str = "leaves you something after the venue's cut and the levy";
    /// `tacsim::hardware::sale::resale::RESALE_OFFERED`'s tail.
    pub const RESALE_OFFERED: &str = "Nothing moves until they accept.";
    /// `tacsim::hardware::sale::resale::RESALE_OFFERED_HULL`'s middle.
    pub const RESALE_OFFERED_HULL: &str = " with her core #";
    /// `tacsim::hardware::sale::resale::RESALE_WITHDRAWN`'s head.
    pub const RESALE_WITHDRAWN: &str = "Your offer of core #";
    /// `tacsim::hardware::sale::resale::RESALE_NO_OWN_OFFER`.
    pub const RESALE_NO_OWN_OFFER: &str = "You have no standing offer of that core.";
    /// `tacsim::hardware::sale::resale::RESALE_NO_OFFER`.
    pub const RESALE_NO_OFFER: &str = "No offer of that stands for you.";
    /// `tacsim::hardware::sale::resale::RESALE_LAPSED`.
    pub const RESALE_LAPSED: &str = "That offer has lapsed; ask the seller to offer it again.";
    /// `tacsim::hardware::sale::resale::RESALE_CHANGED`'s tail.
    pub const RESALE_CHANGED: &str = "what it sells has changed since it was made";
    /// `tacsim::hardware::sale::resale::RESALE_QUOTE`'s middle.
    pub const RESALE_QUOTE: &str = " offers you core #";
    /// `tacsim::hardware::sale::resale::RESALE_QUOTE_HULL`'s tail.
    pub const RESALE_QUOTE_HULL: &str = ", cuts included.";
    /// `tacsim::hardware::sale::resale::RESALE_HOW`'s tail.
    pub const RESALE_HOW: &str = "your money leaves your account only as it passes to you";
    /// `tacsim::hardware::sale::resale::RESALE_SHORT`'s head.
    pub const RESALE_SHORT: &str = "Your credit account cannot cover ";
    /// `tacsim::hardware::sale::resale::RESALE_NO_ACCOUNT`.
    pub const RESALE_NO_ACCOUNT: &str = "You hold no credit account to pay from.";
    /// `tacsim::hardware::sale::resale::RESALE_ACCEPTING`'s head.
    pub const RESALE_ACCEPTING: &str = "You accept the offer of core #";
    /// `tacsim::hardware::sale::resale::RESALE_BOUGHT`'s middle.
    pub const RESALE_BOUGHT: &str = " and its title are yours, for ";
    /// `tacsim::hardware::sale::resale::RESALE_BOUGHT_HULL`'s middle.
    pub const RESALE_BOUGHT_HULL: &str = " are yours, with both titles, for ";
    /// `tacsim::hardware::sale::resale::RESALE_SOLD`'s middle.
    pub const RESALE_SOLD: &str = "; you are paid ";
    /// `tacsim::hardware::sale::resale::RESALE_NO_OFFERS`.
    pub const RESALE_NO_OFFERS: &str = "No core offers stand to or from you.";
    /// `tacsim::hardware::sale::resale::RESALE_OFFER_ROW`'s tail.
    pub const RESALE_OFFER_ROW: &str = " days left to accept it.";
    /// `tacsim::hardware::sale::resale::RESALE_BUSY`.
    pub const RESALE_BUSY: &str = "You are already doing something; try again in a moment.";
}

/// **The scrap sold on the board** (doc 48 cut EX.3, `tacsim::port::bay_work::scrap`): what the
/// holder hears when nothing whole is sold, or a row is given up.
pub mod scrap {
    /// `tacsim::port::bay_work::scrap::NOTHING_WHOLE`.
    pub const NOTHING_WHOLE: &str = "scrap is paid by the whole tonne";
    /// `tacsim::port::bay_work::scrap::SALE_GIVEN_UP`'s middle.
    pub const SALE_GIVEN_UP: &str = "; it stays in your scrap at ";
}

/// **The exchange at the venue** (doc 48 cut EX.1b): a player's locker (`+locker`,
/// `tacsim::port::locker`) and the venue gate's new refusals (`tacsim::port::market`). No lane
/// reads them yet; each is held to the game's own text below.
pub mod venue {
    /// `tacsim::port::locker::LOCKER_OPENED`'s tail: a locker opened.
    pub const LOCKER_OPENED: &str = "Goods you buy here with no hold docked go into it.";
    /// `tacsim::port::locker::LOCKER_ALREADY`'s tail.
    pub const LOCKER_ALREADY: &str = " is already open.";
    /// `tacsim::port::locker::LOCKER_NO_VENUE`.
    pub const LOCKER_NO_VENUE: &str = "There is no market here to keep a locker at.";
    /// `tacsim::port::locker::LOCKER_SHOW`'s tail: what a locker holds and owes.
    pub const LOCKER_SHOW: &str = " t held, charged ";
    /// `tacsim::port::locker::LOCKER_NONE`'s tail.
    pub const LOCKER_NONE: &str = "+locker/open opens one.";
    /// `tacsim::port::market::NOT_AT_VENUE`'s tail: a non-port venue, away from it.
    pub const NOT_AT_VENUE: &str = " market trades where it stands, and you are not there.";
    /// `tacsim::port::market::VENUE_UNPLACED`'s tail: a venue the game places nowhere.
    pub const VENUE_UNPLACED: &str = " market has no place to trade in, so it cannot trade.";
    /// `tacsim::port::market::LOCKER_ELSEWHERE`'s head: another venue's or player's locker.
    pub const LOCKER_ELSEWHERE: &str = "That locker is not yours at ";
}

/// **The way back to a Co-op Dray** (the owner's 2026-11-17 ruling): the recall after a real
/// week idle (168 IC days), its notice and the desk's re-charter (`tacsim::port::coop`,
/// `tacsim::port::recall`). No lane reads them yet; each is held to the game's own text below.
pub mod coop {
    /// `tacsim::port::coop::ENROLLED_ONCE`'s tail: a live charter on the account.
    pub const ENROLLED_ONCE: &str = "the Co-op charters each pilot one Dray at a time.";
    /// `tacsim::port::coop::GRADUATED_ONCE`'s tail: a graduate at the desk.
    pub const GRADUATED_ONCE: &str =
        "the Co-op charters only captains who have no hull of their own.";
    /// `tacsim::port::coop::REVOKED_FINAL`'s tail: a staff-revoked charter.
    pub const REVOKED_FINAL: &str = "the Co-op does not charter you again.";
    /// `tacsim::port::coop::RECHARTER_WAIT`'s tail: the last Dray not back yet.
    pub const RECHARTER_WAIT: &str = "; charter again once she is in its pool.";
    /// `tacsim::port::coop::RECHARTER_HINT`'s head: the way back.
    pub const RECHARTER_HINT: &str =
        "To fly again, charter another Dray: +coop/enroll <ship name> at the ";
    /// `tacsim::port::coop::RECHARTERED_TERMS`'s head: a re-charter's terms.
    pub const RECHARTERED_TERMS: &str = "No second stake: the Co-op stakes a captain once. Buyout ";
    /// `tacsim::port::coop::REGISTRAR_LINE`'s last sentence: the recall and the way back.
    pub const REGISTRAR_LINE: &str =
        "takes its Dray back; you may charter another, with no second stake.";
    /// `tacsim::port::coop::TERMS_ONE_AT_A_TIME`: the `+coop` terms line's last clause.
    pub const TERMS_ONE_AT_A_TIME: &str = "one stake per pilot, and one Dray at a time.";
    /// `tacsim::port::recall::DORMANT_NOTICE`'s tail: the recall after a real week idle.
    pub const DORMANT_NOTICE: &str = "): a real week without a departure. The stake stays yours.";
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
    "jobs::JOB_LINE",
    "jobs::JOB_NEGOTIABLE",
    "jobs::JOB_BOND",
    "jobs::JOB_TAKE",
    "jobs::TERM_BY",
    "jobs::TERM_TODAY",
    "history::HISTORY_ROW",
    "history::HISTORY_NO_PRICE",
    // The crew family's short needles (48e L8, clients:G14: crew joined the table); the
    // crew module's own docs say what a reader uses beside each.
    "crew::OFFER_WAGE",
    "crew::WAGE_LINE",
    "crew::LISTED",
    "crew::BOARD_SUBJECT",
    "crew::BOARD_TAKE",
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
    const SELLER_RS: &str = include_str!("../../../mush-market/src/seller.rs");
    const SERVICES_RS: &str = include_str!("../../../mush-market/src/services.rs");
    const HISTORY_RS: &str = include_str!("../../../mush-market/src/history.rs");
    const HULL_SALE_RS: &str = include_str!("../../../tacsim/src/port/hull_sale.rs");
    const CORE_RESALE_RS: &str = include_str!("../../../tacsim/src/hardware/sale/resale.rs");
    const WAGES_RS: &str = include_str!("../../../tacsim/src/detachment/crew/wages.rs");

    fn crew_rows() -> Vec<Row> {
        use super::crew as bot;
        use tacsim::detachment::crew::wages as game;
        rows!("";
            OFFER_WAGE,
            OFFERS_WAGES,
            OFFERS_ABOARD,
            WAGE_LINE,
            WAGE_NONE,
            LIST_USAGE,
            LISTED,
            LISTING_SHOW,
            NOT_LISTED,
            UNLISTED,
            LIST_AT_PORT,
            BOARD_SUBJECT,
            BOARD_TAKE,
        )
    }
    const DISCHARGE_RS: &str = include_str!("../../../tacsim/src/detachment/crew/discharge.rs");

    fn discharge_rows() -> Vec<Row> {
        use super::discharge as bot;
        use tacsim::detachment::crew::discharge as game;
        rows!("";
            DISCHARGE_USAGE,
            NOT_YOURS,
            AT_PORT,
            ON_SORTIE,
            IN_FIELD,
            CAPTIVE,
            BUSY,
            NOT_NOW,
            DISCHARGED_PAID,
            DISCHARGED_OWING,
            DISCHARGED_PART,
            DISCHARGED_FREE,
            RETURNED,
            STANDING_DEBT,
        )
    }
    const HAUL_RS: &str = include_str!("../../../tacsim/src/services/haul.rs");
    const ESCORT_RS: &str = include_str!("../../../tacsim/src/services/escort.rs");

    fn venue_rows() -> Vec<Row> {
        use super::venue as bot;
        let mut rows = {
            use tacsim::port::locker as game;
            rows!("";
                LOCKER_OPENED,
                LOCKER_ALREADY,
                LOCKER_NO_VENUE,
                LOCKER_SHOW,
                LOCKER_NONE,
            )
        };
        rows.extend({
            use tacsim::port::market as game;
            rows!("";
                NOT_AT_VENUE,
                VENUE_UNPLACED,
                LOCKER_ELSEWHERE,
            )
        });
        rows
    }

    fn job_board_rows() -> Vec<Row> {
        use super::jobs as bot;
        use mush_market::services as game;
        rows!("";
            JOBS_USAGE,
            JOBS_NO_VENUE,
            JOBS_NONE,
            JOBS_HEAD,
            JOB_LINE,
            JOB_NEGOTIABLE,
            JOB_BOND,
            JOB_TAKE,
            TERM_FROM_ACCEPTANCE,
            TERM_BY,
            TERM_TODAY,
            TERM_ON_PERFORMANCE,
            TERM_DAILY,
            JOBS_MORE,
        )
    }

    fn jobs_rows() -> Vec<Row> {
        use super::jobs as bot;
        let mut rows = job_board_rows();
        rows.extend({
            use tacsim::surface::retrieval as game;
            rows!("";
                ACCEPT_USAGE,
                BOND_CLAUSE,
                NOT_FOR_ORG,
                ORG_NO_ACCOUNT,
            )
        });
        rows
    }

    fn coop_rows() -> Vec<Row> {
        use super::coop as bot;
        let mut rows = {
            use tacsim::port::coop as game;
            rows!("";
                ENROLLED_ONCE,
                GRADUATED_ONCE,
                REVOKED_FINAL,
                RECHARTER_WAIT,
                RECHARTER_HINT,
                RECHARTERED_TERMS,
                REGISTRAR_LINE,
                TERMS_ONE_AT_A_TIME,
            )
        };
        rows.extend({
            use tacsim::port::recall as game;
            rows!("";
                DORMANT_NOTICE,
            )
        });
        rows
    }

    fn history_rows() -> Vec<Row> {
        use super::history as bot;
        use mush_market::history as game;
        rows!("";
            HISTORY_USAGE,
            HISTORY_NO_VENUE,
            HISTORY_NONE,
            HISTORY_HEAD,
            HISTORY_ROW,
            HISTORY_THIS_WEEK,
            HISTORY_WEEKS_AGO,
            HISTORY_LAST_WEEK,
            HISTORY_NO_PRICE,
        )
    }

    fn exchange_rows() -> Vec<Row> {
        use super::exchange as bot;
        use mush_market::seller as game;
        rows!("";
            ASK_POSTED,
            POSTED_AWAY,
            BID_POSTED,
            FOR_ONLY,
            PRIVATE_MARK,
            BID_SUPPLY,
            WITHDRAWN,
            BID_WITHDRAWN,
            MINE_HEAD,
            MINE_NONE,
            MINE_COUNT,
            NO_SUCH_LOT,
            NO_SUCH_VAULT,
            LOT_HELD,
            NOT_A_LOT_GOOD,
            LOT_TOO_BIG,
            SELLER_FULL,
            NAME_VENUE,
            NO_CURRENCY,
            NO_ACCOUNT,
            PRIVATE_NAMES_OTHER,
            PRIVATE_TO_OTHER,
            OWN_LISTING,
            BID_TWICE,
            BID_INTO_HOLD,
            BID_CANNOT_COVER,
            BID_NEEDS_APPROVAL,
            BID_NOT_HELD,
            BIDDER_NOT_BUYING,
            SELLER_GONE,
            POST_USAGE,
            BID_USAGE,
            WITHDRAW_USAGE,
            NO_SUCH_PLAYER,
            ASSET_POSTED,
            NO_SUCH_ASSET,
            NOT_LISTABLE,
            NOT_FOR_ORG,
            ALREADY_FOR_SALE,
        )
    }

    fn scrap_rows() -> Vec<Row> {
        use super::scrap as bot;
        use tacsim::port::bay_work::scrap as game;
        rows!("";
            NOTHING_WHOLE,
            SALE_GIVEN_UP,
        )
    }

    fn hull_sale_rows() -> Vec<Row> {
        use super::hull_sale as bot;
        use tacsim::port::hull_sale as game;
        rows!("";
            HULL_CORED,
            HULL_HOLD_LADEN,
            HULL_HOLD_DARK,
            HULL_IN_BAY,
            HULL_RIG_ABOARD,
        )
    }

    fn haul_rows() -> Vec<Row> {
        use super::haul as bot;
        use tacsim::services::haul as game;
        rows!("";
            HAUL_USAGE,
            POST_PREVIEW,
            POSTED,
            NO_VENUE,
            NO_LOCKER,
            SHORT,
            NO_GOOD,
            NO_PORT,
            SAME_PORT,
            OVER_CAP,
            NO_REFERENCE,
            POST_FAILED,
            NO_SUCH_JOB,
            OWN_JOB,
            NOT_DOCKED,
            NO_ROOM,
            GOODS_GONE,
            ACCEPT_PREVIEW,
            ACCEPTED,
            ACCEPT_FAILED,
            TOLD_TAKEN,
            NOT_THERE,
            TOO_LATE,
            NOT_ABOARD,
            DELIVER_PREVIEW,
            DELIVERING,
            TOLD_DELIVERED,
            TOLD_RETURNED,
            NOT_WITHDRAWABLE,
            WITHDRAWN,
            NONE,
            HEAD,
            ROW,
        )
    }
    const HAUL_EXCLUDED: &[(&str, &str)] = &[
        ("HAUL_VERB", "the verb's name"),
        ("HAUL_PREFIX", "the record's key prefix, never printed"),
        ("HAUL_ACTION", "the legs' action id, never printed"),
        ("HAUL_OUTCOME", "the legs' outcome id, never printed"),
        ("HAUL_FAMILY", "the ledger's family name, never printed"),
        (
            "HAUL_HELP",
            "the verb's help, read through help, not answered",
        ),
        ("CHANNEL", "the channel lines go out on, never printed"),
    ];

    fn escort_rows() -> Vec<Row> {
        use super::escort as bot;
        use tacsim::services::escort as game;
        rows!("";
            ESCORT_USAGE,
            POST_PREVIEW,
            POSTED,
            NOT_YOURS,
            NOT_DOCKED,
            NO_PORT,
            SAME_PORT,
            NO_VENUE,
            ALREADY,
            POST_FAILED,
            NO_SUCH_JOB,
            OWN_JOB,
            SAME_HULL,
            ACCEPT_PREVIEW,
            BOND_CLAUSE,
            ACCEPTED,
            ACCEPT_FAILED,
            TOLD_TAKEN,
            TOLD_STARTED,
            TOLD_KEPT,
            TOLD_FAILED,
            WHY_WRECKED,
            WHY_CAPTURED,
            WHY_LEFT,
            WHY_FAR,
            NOT_WITHDRAWABLE,
            WITHDRAWN,
            NONE,
            HEAD,
            ROW,
        )
    }
    const ESCORT_EXCLUDED: &[(&str, &str)] = &[
        ("ESCORT_VERB", "the verb's name"),
        ("ESCORT_PREFIX", "the record's key prefix, never printed"),
        ("PROVIDER", "the fact's namespace, never printed"),
        ("FACT_KEPT", "the fact's kind, never printed"),
        (
            "ESCORT_HELP",
            "the verb's help, read through help, not answered",
        ),
        ("CHANNEL", "the channel lines go out on, never printed"),
    ];

    fn core_resale_rows() -> Vec<Row> {
        use super::core_resale as bot;
        use tacsim::hardware::sale::resale as game;
        rows!("";
            RESALE_OFFER_USAGE,
            RESALE_ACCEPT_USAGE,
            RESALE_OFF,
            RESALE_NOT_CORE,
            RESALE_NOT_YOURS,
            RESALE_NOT_HELD,
            RESALE_FITTED,
            RESALE_DESTROYED,
            RESALE_NO_PLAYER,
            RESALE_SELF,
            RESALE_PRICE,
            RESALE_OFFERED,
            RESALE_OFFERED_HULL,
            RESALE_WITHDRAWN,
            RESALE_NO_OWN_OFFER,
            RESALE_NO_OFFER,
            RESALE_LAPSED,
            RESALE_CHANGED,
            RESALE_QUOTE,
            RESALE_QUOTE_HULL,
            RESALE_HOW,
            RESALE_SHORT,
            RESALE_NO_ACCOUNT,
            RESALE_ACCEPTING,
            RESALE_BOUGHT,
            RESALE_BOUGHT_HULL,
            RESALE_SOLD,
            RESALE_NO_OFFERS,
            RESALE_OFFER_ROW,
            RESALE_BUSY,
        )
    }

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
            CLAIM_ADMITTED,
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
            PUT_ABOARD,
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
        // 48e L8 (clients:G14): crew and discharge joined the cross-family checks.
        let families: [(&str, Vec<Row>); 20] = [
            ("end_user", end_user_rows()),
            ("renewal", renewal_rows()),
            ("claim", claim_rows()),
            ("dispatch", dispatch_rows()),
            ("absent", absent_rows()),
            ("market", market_rows()),
            ("ship_mind", ship_mind_rows()),
            ("practice", practice_rows()),
            ("exchange", exchange_rows()),
            ("jobs", jobs_rows()),
            ("history", history_rows()),
            ("hull_sale", hull_sale_rows()),
            ("core_resale", core_resale_rows()),
            ("haul", haul_rows()),
            ("escort", escort_rows()),
            ("venue", venue_rows()),
            ("scrap", scrap_rows()),
            ("coop", coop_rows()),
            ("crew", crew_rows()),
            ("discharge", discharge_rows()),
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
    /// `hull_sale.rs`.
    const HULL_SALE_EXCLUDED: &[(&str, &str)] = &[
        (
            "HULL_PACK",
            "the hulls' catalog pack name, not a sentence the game prints",
        ),
        (
            "HULL_CODE_PREFIX",
            "a hull entry's catalog-code prefix, not a sentence",
        ),
    ];
    /// `hardware/sale/resale.rs` (doc 48 unit EX.2b).
    const CORE_RESALE_EXCLUDED: &[(&str, &str)] = &[
        ("OFFER_ACTION", "the offer's action id, never printed"),
        ("OFFER_OUTCOME", "the offer's outcome id, never printed"),
        ("RESALE_ACTION", "the acceptance's action id, never printed"),
        (
            "RESALE_OUTCOME",
            "the acceptance's outcome id, never printed",
        ),
        (
            "OFFER_FAMILY",
            "the offers' store family name, never printed",
        ),
        (
            "RESALE_HELP",
            "the verb's help text, read by help, not a line the bot waits on",
        ),
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
            "the same text, \"You hold no license {licence}.\", for a launch's mind= and for +minds",
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
        (
            "history::HISTORY_NO_PRICE",
            "haul::NO_LOCKER",
            "a partial needle, read after \"ask \" or \"bid \" in a +market/history row",
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

    // Doc 48 cut EX.1a: the exchange's personal seller.
    #[test]
    fn exchange_needles_are_in_step() {
        in_step("exchange", &exchange_rows());
    }
    #[test]
    fn exchange_census() {
        census("seller.rs", SELLER_RS, &exchange_rows(), &[]);
    }
    #[test]
    fn exchange_needles_are_distinct() {
        distinct(&qualified("exchange", exchange_rows()), &[]);
    }
    // Doc 48 cut EX.8: the price history.
    #[test]
    fn history_needles_are_in_step() {
        in_step("history", &history_rows());
    }
    #[test]
    fn history_census() {
        census("history.rs", HISTORY_RS, &history_rows(), &[]);
    }
    #[test]
    fn history_needles_are_distinct() {
        distinct(&qualified("history", history_rows()), &[]);
    }
    // Doc 48 cut EX.5: the job board.
    #[test]
    fn jobs_needles_are_in_step() {
        in_step("jobs", &jobs_rows());
    }
    #[test]
    fn jobs_census() {
        census("services.rs", SERVICES_RS, &job_board_rows(), &[]);
    }
    // Doc 48 cut EX.7: crew wages and crew for hire.
    #[test]
    fn crew_needles_are_in_step() {
        in_step("crew", &crew_rows());
    }
    #[test]
    fn crew_census() {
        census(
            "wages.rs",
            WAGES_RS,
            &crew_rows(),
            &[
                ("WAGE_NS_PREFIX", "a namespace, never shown"),
                ("LISTING_ATTR", "an attribute name, never shown"),
            ],
        );
    }
    #[test]
    fn crew_needles_are_distinct() {
        distinct(&qualified("crew", crew_rows()), &[]);
    }
    // Doc 48 cut EX.7b: a discharge at a port.
    #[test]
    fn discharge_needles_are_in_step() {
        in_step("discharge", &discharge_rows());
    }
    #[test]
    fn discharge_census() {
        census(
            "discharge.rs",
            DISCHARGE_RS,
            &discharge_rows(),
            &[("NPC_TOLD", "said to the NPC's own role, never to a player")],
        );
    }
    #[test]
    fn discharge_needles_are_distinct() {
        distinct(&qualified("discharge", discharge_rows()), &[]);
    }
    #[test]
    fn jobs_needles_are_distinct() {
        distinct(&qualified("jobs", jobs_rows()), &[]);
    }
    // Doc 48 cut EX.2: a hull listed on the exchange.
    #[test]
    fn hull_sale_needles_are_in_step() {
        in_step("hull_sale", &hull_sale_rows());
    }
    #[test]
    fn hull_sale_census() {
        census(
            "hull_sale.rs",
            HULL_SALE_RS,
            &hull_sale_rows(),
            HULL_SALE_EXCLUDED,
        );
    }
    #[test]
    fn hull_sale_needles_are_distinct() {
        distinct(&qualified("hull_sale", hull_sale_rows()), &[]);
    }
    // Doc 48 cut EX.6: the haul.
    #[test]
    fn haul_needles_are_in_step() {
        in_step("haul", &haul_rows());
    }
    #[test]
    fn haul_census() {
        census("services/haul.rs", HAUL_RS, &haul_rows(), HAUL_EXCLUDED);
    }
    #[test]
    fn haul_needles_are_distinct() {
        distinct(&qualified("haul", haul_rows()), &[]);
    }
    // Doc 48 cut EX.6: the escort.
    #[test]
    fn escort_needles_are_in_step() {
        in_step("escort", &escort_rows());
    }
    #[test]
    fn escort_census() {
        census(
            "services/escort.rs",
            ESCORT_RS,
            &escort_rows(),
            ESCORT_EXCLUDED,
        );
    }
    #[test]
    fn escort_needles_are_distinct() {
        distinct(&qualified("escort", escort_rows()), &[]);
    }
    // Doc 48 unit EX.2b: a governed core resold to a player.
    #[test]
    fn core_resale_needles_are_in_step() {
        in_step("core_resale", &core_resale_rows());
    }
    #[test]
    fn core_resale_census() {
        census(
            "hardware/sale/resale.rs",
            CORE_RESALE_RS,
            &core_resale_rows(),
            CORE_RESALE_EXCLUDED,
        );
    }
    #[test]
    fn core_resale_needles_are_distinct() {
        distinct(&qualified("core_resale", core_resale_rows()), &[]);
    }
    // Doc 48 cut EX.3: the scrap sold on the board.
    #[test]
    fn scrap_needles_are_in_step() {
        in_step("scrap", &scrap_rows());
    }
    #[test]
    fn scrap_needles_are_distinct() {
        distinct(&qualified("scrap", scrap_rows()), &[]);
    }
    // Doc 48 cut EX.1b: the exchange at the venue.
    #[test]
    fn venue_needles_are_in_step() {
        in_step("venue", &venue_rows());
    }
    #[test]
    fn venue_needles_are_distinct() {
        distinct(&qualified("venue", venue_rows()), &[]);
    }
    /// Every venue sentence fits the 120-column floor with its longest venue word filled in.
    #[test]
    fn venue_sentences_fit_120_columns() {
        for (name, sentence, _) in venue_rows() {
            let filled = sentence
                .replace("{venue}", "launch.counter")
                .replace("{port}", "Stickney Yards")
                .replace("{tonnes}", "20.0")
                .replace("{owed}", "1,234,567.89 cr");
            assert!(filled.chars().count() < 120, "venue::{name}: {filled}");
        }
    }

    // The owner's 2026-11-17 ruling: the way back to a Co-op Dray.
    #[test]
    fn coop_needles_are_in_step() {
        in_step("coop", &coop_rows());
    }
    #[test]
    fn coop_needles_are_distinct() {
        distinct(&qualified("coop", coop_rows()), &[]);
    }
    /// Every Co-op sentence fits the 120-column floor with a long ship name, the mark, the desk
    /// and the port filled in, one sentence at a time (of the registrar's line, its last).
    #[test]
    fn coop_sentences_fit_120_columns() {
        for (name, sentence, _) in coop_rows() {
            let filled = sentence
                .replace("{name}", "Patience Wen of Kirkwood")
                .replace("{mark}", "FPC-D-0408")
                .replace("{office}", "Registry & Co-op Desk")
                .replace("{port}", "Ceres Freeport")
                .replace("{buyout}", "90,000.00cr")
                .replace("{countersign}", "Ilse Oduya-Brandt");
            // The registrar's line is several sentences; only its last is this ruling's.
            let lines: Vec<&str> = filled.split_inclusive(". ").collect();
            let checked: Vec<&str> = if name == "REGISTRAR_LINE" {
                lines.last().copied().into_iter().collect()
            } else {
                lines
            };
            for line in checked {
                assert!(line.chars().count() < 120, "coop::{name}: {line}");
            }
        }
    }

    /// Every exchange sentence fits the 120-column floor as written (a filled line is held to
    /// it by `mush-market`'s own `ex1a_every_new_sentence_fits_120_columns`).
    #[test]
    fn exchange_sentences_fit_120_columns() {
        for (name, sentence, _) in exchange_rows() {
            assert!(
                sentence.chars().count() < 120,
                "exchange::{name}: {sentence}"
            );
        }
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

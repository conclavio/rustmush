//! 17g C2d (C2 plan §2.1's renewal exception, §2.6): the renewal services'
//! player steps as one public front. `@mindrenewal` and `@housemindrenewal`
//! keep their own switches and print exactly their pre-C2d sentences; TacSim's
//! `+minds/renew` takes the same steps (since 17g C2h through each service's
//! `front`, below; C2d's direct `player_step` call could not read the
//! market's handles from a TacSim context). Every spelling reaches one
//! surface underneath: the service's own admission,
//! retained entry, idempotency key and continuation, which still grants or
//! refunds a funded offer by itself. Nothing here adds an authority: the step
//! acts only on the calling actor's own licence (the renewal reader's own
//! holder check), and every refusal is the service's one uniform line.
//!
//! The House service shares these types (its shapes are equal).
//!
//! **The player front (17g C2h, the lead's ruling R3, the C2e plan §5.1).**
//! TacSim's `+minds/renew` cannot call `player_step` itself: its command
//! context reads as TacSim, while each service reads through the market's own
//! renewal read and typed families. So each service registers one more
//! switch, `front` (`@mindrenewal/front` and `@housemindrenewal/front`, not
//! documented to players, R5), mints a continuation on it and delegates that
//! one continuation to the module its configuration names (TacSim) at
//! composition (`mush_core::Registrar::delegate_continuation`). TacSim's
//! handler checks its own synchronous refusals and hands the typed step over
//! with [`front_argument`]; mush-core runs the market's handler, under the
//! market's identity, for the same player, and only for the player whose own
//! typed line it is dispatching. The market arm reads the licence through the
//! service's own holder check, takes the step and THEN prints the `+minds`
//! words below, so a step that submitted nothing never reads as a request.
//! The words are TacSim's C2d sentences, verbatim, moved here because only
//! the market's operation row carries the offer's frozen price and expiry
//! (Roll's mirror carries neither).

/// One player step, as `+minds/renew` asks for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenewalPlayerStep {
    /// The licence's renewal state and, when one exists, its last offer.
    Status,
    /// Ask for an offer: the service's `request` branch while the current
    /// term has no attempt, else its `retry` branch when the last operation
    /// is retryable (a closed or refunded offer).
    Request,
    /// Pay the drafted offer into escrow (the E4 consent).
    Accept,
    /// Close the drafted offer.
    Decline,
}

/// What a step did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenewalPlayerOutcome {
    /// `Status`'s answer.
    Status(RenewalStatusView),
    /// One submission was made.
    Queued,
    /// Nothing was submitted: the service's retained entry for this
    /// selection still awaits its earlier submission's terminal, or (17g C2h,
    /// the C2e plan §5.1) the actor already has an authoritative submission
    /// awaiting its receipt, so the service sent nothing (before C2h this
    /// last case read `Queued`, a line no submission backed).
    Unresolved,
}

/// The renewal state of a licence's current term, as the status branch reads
/// it: no operation yet, or the last operation's phase. Refunded and both
/// unfunded closures are `Closed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenewalState {
    None,
    Drafted,
    Funded,
    Granted,
    Closed,
}

/// The status branch's facts, read exactly as `@…renewal/status` reads them:
/// the current term's end (the service's due rule is `world_ic_now().at <
/// term_end` for "not due"), and, when an operation exists, its frozen offer
/// terms. No account, digest, binding or other holder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenewalStatusView {
    state: RenewalState,
    term_end_ic: i64,
    offer: Option<RenewalOfferTerms>,
}

/// The last operation's frozen offer and binding terms.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RenewalOfferTerms {
    pub(crate) price_minor: i64,
    pub(crate) instrument: String,
    pub(crate) expires_ic: i64,
    pub(crate) term_seconds: i64,
    pub(crate) grace_seconds: i64,
    pub(crate) registry_permille: u16,
    pub(crate) venue_permille: u16,
    pub(crate) parent_permille: u32,
}

impl RenewalStatusView {
    pub(crate) const fn new(
        state: RenewalState,
        term_end_ic: i64,
        offer: Option<RenewalOfferTerms>,
    ) -> Self {
        Self {
            state,
            term_end_ic,
            offer,
        }
    }
    #[must_use]
    pub const fn state(&self) -> RenewalState {
        self.state
    }
    /// The current term's end, in world-IC seconds.
    #[must_use]
    pub const fn term_end_ic(&self) -> i64 {
        self.term_end_ic
    }
    /// The last offer's frozen price in minor units.
    #[must_use]
    pub fn price_minor(&self) -> Option<i64> {
        self.offer.as_ref().map(|o| o.price_minor)
    }
    /// The last offer's instrument.
    #[must_use]
    pub fn instrument(&self) -> Option<&str> {
        self.offer.as_ref().map(|o| o.instrument.as_str())
    }
    /// When the last offer closes, in world-IC seconds.
    #[must_use]
    pub fn offer_expires_ic(&self) -> Option<i64> {
        self.offer.as_ref().map(|o| o.expires_ic)
    }
    /// The renewed term's length, in IC seconds.
    #[must_use]
    pub fn term_seconds(&self) -> Option<i64> {
        self.offer.as_ref().map(|o| o.term_seconds)
    }
    /// The renewed term's Grace, in IC seconds.
    #[must_use]
    pub fn grace_seconds(&self) -> Option<i64> {
        self.offer.as_ref().map(|o| o.grace_seconds)
    }
    #[must_use]
    pub fn registry_permille(&self) -> Option<u16> {
        self.offer.as_ref().map(|o| o.registry_permille)
    }
    #[must_use]
    pub fn venue_permille(&self) -> Option<u16> {
        self.offer.as_ref().map(|o| o.venue_permille)
    }
    /// The sum of the parent royalties (always 0 for a House Pin).
    #[must_use]
    pub fn parent_royalty_permille(&self) -> Option<u32> {
        self.offer.as_ref().map(|o| o.parent_permille)
    }
}

/// The switches a step selects, as the service command reads its own.
/// `status` is "status, or no switch at all"; the others are tested in the
/// service command's own order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Switches {
    pub(crate) status: bool,
    pub(crate) request: bool,
    pub(crate) accept: bool,
    pub(crate) decline: bool,
    pub(crate) retry: bool,
}
impl Switches {
    /// `player_step`'s selection: `Request` sets both `request` and `retry`,
    /// so the service's request branch runs while the term is unattempted
    /// and its retry branch after a retryable closure.
    pub(crate) const fn of(step: RenewalPlayerStep) -> Self {
        let none = Self {
            status: false,
            request: false,
            accept: false,
            decline: false,
            retry: false,
        };
        match step {
            RenewalPlayerStep::Status => Self {
                status: true,
                ..none
            },
            RenewalPlayerStep::Request => Self {
                request: true,
                retry: true,
                ..none
            },
            RenewalPlayerStep::Accept => Self {
                accept: true,
                ..none
            },
            RenewalPlayerStep::Decline => Self {
                decline: true,
                ..none
            },
        }
    }
    /// The service command's own switches, unchanged since before C2d.
    pub(crate) fn of_command(switches: &mush_core::SwitchSet) -> Self {
        Self {
            status: switches.has("status") || switches.is_empty(),
            request: switches.has("request"),
            accept: switches.has("accept"),
            decline: switches.has("decline"),
            retry: switches.has("retry"),
        }
    }
}

/// `@…renewal`'s line after a submission.
pub(crate) const QUEUED_LINE: &str =
    "Renewal request queued. Status or its acknowledged receipt establishes completion.";
/// `@…renewal`'s line while the selection's earlier submission is unresolved.
pub(crate) const UNRESOLVED_LINE: &str =
    "A renewal request is still unresolved; inspect status before retrying.";

/// The status label of each state.
const fn label(state: RenewalState) -> &'static str {
    match state {
        RenewalState::None => "No renewal offer has been requested.",
        RenewalState::Drafted => "A renewal offer awaits your explicit acceptance or decline.",
        RenewalState::Funded => "Payment is held; completion is pending.",
        RenewalState::Granted => "Renewal completed.",
        RenewalState::Closed => {
            "The last renewal closed without a new term. You may explicitly request a retry."
        }
    }
}

/// `@mindrenewal/status`'s sentence, byte for byte as before C2d.
pub(crate) fn personal_status_line(view: &RenewalStatusView) -> String {
    let details = view.offer.as_ref().map(|o| format!(" Price: {} minor units of {}; offer expires at world-IC {}. Term: {} seconds plus {} seconds grace. Registry cut: {} per mille; venue cut: {} per mille; parent royalties: {} per mille.",
        o.price_minor, o.instrument, o.expires_ic, o.term_seconds, o.grace_seconds,
        o.registry_permille, o.venue_permille, o.parent_permille)).unwrap_or_default();
    format!("{}{details}", label(view.state))
}

/// `@housemindrenewal/status`'s sentence, byte for byte as before C2d (no
/// parent royalties: a House Pin has none).
pub(crate) fn house_status_line(view: &RenewalStatusView) -> String {
    let details = view.offer.as_ref().map(|o| format!(" Price: {} minor units of {}; offer expires at world-IC {}. Term: {} seconds plus {} seconds grace. Registry cut: {} per mille; venue cut: {} per mille.",
        o.price_minor, o.instrument, o.expires_ic, o.term_seconds, o.grace_seconds,
        o.registry_permille, o.venue_permille)).unwrap_or_default();
    format!("{}{details}", label(view.state))
}

/// The service command's one line for an outcome.
pub(crate) fn outcome_line(
    outcome: RenewalPlayerOutcome,
    status: fn(&RenewalStatusView) -> String,
) -> String {
    match outcome {
        RenewalPlayerOutcome::Status(view) => status(&view),
        RenewalPlayerOutcome::Queued => QUEUED_LINE.to_owned(),
        RenewalPlayerOutcome::Unresolved => UNRESOLVED_LINE.to_owned(),
    }
}

// 17g C2h (the C2e plan §5.1): the `+minds/renew` words, verbatim from the
// C2d brief (they were TacSim's `market_commands::words` until C2h). The
// market's `front` arm prints them after the step, through `Output::info`,
// the line TacSim printed them with. `{licence}` is the typed licence
// number; `{price}` the credit rendering (`credits`); `{days}`, `{term}` and
// `{grace}` are whole IC days and `{hours}` whole IC hours, rounded up.

/// The licence's current term has not ended (the service's due rule).
pub const RENEW_NOT_DUE: &str =
    "Licence {licence} is not due for renewal; its term has {days} IC days left.";
/// A renewal offer was asked for (the step submitted the Draft).
pub const RENEWING: &str = "Asking for a renewal offer on licence {licence}. +minds/renew {licence} shows it once it is confirmed.";
/// A drafted offer's terms.
pub const RENEW_OFFER: &str = "Licence {licence} renews for {price} ({term}+{grace} IC days); the offer is open {hours} more IC hours.";
/// How to answer a drafted offer.
pub const RENEW_HOW: &str =
    "Pay it with +minds/renew/pay {licence}, or turn it down with +minds/renew/decline {licence}.";
/// The holder's payment is held; the service completes the renewal.
pub const RENEW_FUNDED: &str =
    "Your renewal payment for licence {licence} is held; the renewal completes on its own.";
/// The holder paid the drafted offer (the step submitted the Fund).
pub const RENEW_PAYING: &str =
    "Paying {price} for the renewal of licence {licence} into escrow; it completes on its own.";
/// The holder turned the drafted offer down (the step submitted the Close).
pub const RENEW_DECLINING: &str = "Turning down the renewal offer on licence {licence}.";

const HOUR: i64 = 3_600;
const DAY: i64 = 86_400;

/// The launch's credit rendering (TacSim's `launch::credits`, which the
/// TacSim offer-line test compares with this): whole credits as `400cr`,
/// else two decimals, `12.50cr`.
#[must_use]
pub fn credits(minor: i64) -> String {
    let (whole, cents) = (minor.div_euclid(100), minor.rem_euclid(100));
    if cents == 0 {
        format!("{whole}cr")
    } else {
        format!("{whole}.{cents:02}cr")
    }
}

/// Whole IC days in `seconds`, rounded up, never below 0.
#[must_use]
pub fn days(seconds: i64) -> i64 {
    seconds.max(0).saturating_add(DAY - 1) / DAY
}

/// Whole hours from `now` to `until`, rounded up, never below 0.
#[must_use]
pub fn hours_left(until: i64, now: i64) -> i64 {
    until.saturating_sub(now).max(0).saturating_add(HOUR - 1) / HOUR
}

/// `template` with each `{key}` replaced by its value, in one pass, so a
/// value is never itself substituted (the values here are server-built
/// numbers and prices).
fn fill(template: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let (before, tail) = rest.split_at(open);
        out.push_str(before);
        let found = tail.find('}').and_then(|close| {
            let key = tail.get(1..close)?;
            let (_, value) = values.iter().find(|(name, _)| *name == key)?;
            Some((*value, close))
        });
        match found {
            Some((value, close)) => {
                out.push_str(value);
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

/// A drafted offer's two lines from its raw fields (the frozen price in
/// minor units, the renewed term and Grace in IC seconds, the offer's IC
/// expiry): `RENEW_OFFER` (term and Grace in whole IC days, the IC hours
/// left rounded up, never below 0) and `RENEW_HOW`.
#[must_use]
pub fn offer_text(
    licence: &str,
    price_minor: i64,
    term_seconds: i64,
    grace_seconds: i64,
    expires_ic: i64,
    now_ic: i64,
) -> [String; 2] {
    let price = credits(price_minor);
    let term = days(term_seconds).to_string();
    let grace = days(grace_seconds).to_string();
    let hours = hours_left(expires_ic, now_ic).to_string();
    [
        fill(
            RENEW_OFFER,
            &[
                ("licence", licence),
                ("price", &price),
                ("term", &term),
                ("grace", &grace),
                ("hours", &hours),
            ],
        ),
        fill(RENEW_HOW, &[("licence", licence)]),
    ]
}

/// `offer_text` over a status view's last offer.
#[must_use]
pub fn offer_lines(licence: &str, view: &RenewalStatusView, now_ic: i64) -> [String; 2] {
    offer_text(
        licence,
        view.price_minor().unwrap_or_default(),
        view.term_seconds().unwrap_or_default(),
        view.grace_seconds().unwrap_or_default(),
        view.offer_expires_ic().unwrap_or(now_ic),
        now_ic,
    )
}

/// The step `+minds/renew` hands to a service's `front` (17g C2h): ask for
/// an offer (or show the one that exists), or pay or decline the drafted
/// offer whose operation the holder's own paid holding names. The operation
/// binds the payment to the offer the holder saw: a changed offer refuses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrontStep {
    /// `+minds/renew <licence>`.
    Request,
    /// `+minds/renew/pay <licence>`, on this drafted operation.
    Accept(mush_core::CommitToken),
    /// `+minds/renew/decline <licence>`, on this drafted operation.
    Decline(mush_core::CommitToken),
}

/// The `front` switch's argument for `step` on `licence`:
/// `<licence> request`, `<licence> accept <operation>` or
/// `<licence> decline <operation>` (the operation's 32-character token
/// text). A typed `@mindrenewal/front` line with the same text is the same
/// step for the same actor, a spelling with no authority of its own.
#[must_use]
pub fn front_argument(licence: mush_roll::commerce::LicenceId, step: FrontStep) -> String {
    match step {
        FrontStep::Request => format!("{} request", licence.get()),
        FrontStep::Accept(operation) => format!("{} accept {operation}", licence.get()),
        FrontStep::Decline(operation) => format!("{} decline {operation}", licence.get()),
    }
}

/// `front_argument`'s inverse, strictly: a decimal licence exactly as `u64`
/// prints it, one of the three words, the operation's exact token text for
/// `accept` and `decline`, single spaces and nothing else. Anything else is
/// `None` (the service's uniform refusal).
pub(crate) fn parse_front(text: &str) -> Option<(mush_roll::commerce::LicenceId, FrontStep)> {
    let words: Vec<&str> = text.split(' ').collect();
    let (number, rest) = words.split_first()?;
    let n = number.parse::<u64>().ok()?;
    if n.to_string() != *number {
        return None;
    }
    let licence = mush_roll::commerce::LicenceId::new(n).ok()?;
    let operation = |token: &str| {
        mush_core::CommitToken::from_hex(token)
            .ok()
            .filter(|t| t.to_string() == token)
    };
    let step = match rest {
        ["request"] => FrontStep::Request,
        ["accept", token] => FrontStep::Accept(operation(token)?),
        ["decline", token] => FrontStep::Decline(operation(token)?),
        _ => return None,
    };
    Some((licence, step))
}

/// What the `front` arm's `request` does after the status read, before any
/// step (17g C2h, the C2e plan §5.1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum FrontAnswer {
    /// Print these lines and submit nothing: a drafted offer's
    /// `RENEW_OFFER` and `RENEW_HOW`, or a funded one's `RENEW_FUNDED`.
    Say(Vec<String>),
    /// Refuse with this line and submit nothing (`RENEW_NOT_DUE`).
    Refuse(String),
    /// Take the request-or-retry step; `RENEWING` once it submitted.
    Ask,
}

/// `request`'s answer from the status view: a drafted offer is shown, a
/// funded one reported; with none, or after the last one closed or granted,
/// `RENEW_NOT_DUE` while the term runs (the service's own due rule,
/// `world_ic_now().at < term_end`), else ask.
pub(crate) fn front_request(licence: &str, view: &RenewalStatusView, now_ic: i64) -> FrontAnswer {
    match view.state() {
        RenewalState::Drafted => FrontAnswer::Say(offer_lines(licence, view, now_ic).into()),
        RenewalState::Funded => FrontAnswer::Say(vec![fill(RENEW_FUNDED, &[("licence", licence)])]),
        RenewalState::None | RenewalState::Closed | RenewalState::Granted => {
            if now_ic < view.term_end_ic() {
                let left = days(view.term_end_ic().saturating_sub(now_ic)).to_string();
                FrontAnswer::Refuse(fill(
                    RENEW_NOT_DUE,
                    &[("licence", licence), ("days", &left)],
                ))
            } else {
                FrontAnswer::Ask
            }
        }
    }
}

/// The line a submitting `front` step prints: `RENEWING` for `Request`,
/// `RENEW_PAYING` with the drafted row's frozen price for `Accept`,
/// `RENEW_DECLINING` for `Decline`.
pub(crate) fn front_line(licence: &str, step: FrontStep, price_minor: i64) -> String {
    match step {
        FrontStep::Request => fill(RENEWING, &[("licence", licence)]),
        FrontStep::Accept(_) => fill(
            RENEW_PAYING,
            &[("price", &credits(price_minor)), ("licence", licence)],
        ),
        FrontStep::Decline(_) => fill(RENEW_DECLINING, &[("licence", licence)]),
    }
}

/// `line` when the step submitted (`Queued`); `None` otherwise, so the arm
/// prints the service's uniform refusal and never a request no submission
/// backs.
pub(crate) fn front_submitted(outcome: &RenewalPlayerOutcome, line: String) -> Option<String> {
    matches!(outcome, RenewalPlayerOutcome::Queued).then_some(line)
}

/// Whether a `front` step names the operation the holder may act on:
/// `Request` always; `Accept` and `Decline` only when their operation is
/// exactly the licence's last operation (`last`), so a payment is bound to
/// the offer the holder saw and a changed offer refuses.
pub(crate) fn front_bound(step: FrontStep, last: Option<mush_core::CommitToken>) -> bool {
    match step {
        FrontStep::Request => true,
        FrontStep::Accept(operation) | FrontStep::Decline(operation) => last == Some(operation),
    }
}

/// What a renewal service's `submit` did (17g C2h, the C2e plan §5.1,
/// review AD-F5). It is `#[must_use]`: a player step that dropped it
/// (`submit(..)?;`, the pre-C2h spelling, followed by an unconditional
/// `Queued`) would report a submission nothing backs, so that spelling is an
/// `unused_must_use` warning, which the workspace's `-D warnings` refuses.
/// The services' own continuation keeps it as `let _sent` (an entry that
/// sent nothing is retried on a later tick).
#[must_use = "a player step reports `Queued` only for a submission that was sent (17g C2h)"]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Submission {
    /// The one submission was made.
    Sent,
    /// Nothing was sent (`Submission::may_send` refused).
    Nothing,
}
impl Submission {
    /// Whether a service's `submit` may send: never while the retained entry
    /// still awaits an earlier attempt's terminal (`attempt_pending`), and
    /// never while the actor already has an authoritative submission awaiting
    /// its receipt (`authoritative_pending`). Both services' `submit` decide
    /// by this, and only by this.
    pub(crate) const fn may_send(attempt_pending: bool, authoritative_pending: bool) -> bool {
        !attempt_pending && !authoritative_pending
    }
}

/// A submitting step's outcome from what the service's `submit` did (17g
/// C2h): `Queued` after the one submission, `Unresolved` when nothing was
/// sent.
pub(crate) const fn sent_outcome(submission: Submission) -> RenewalPlayerOutcome {
    match submission {
        Submission::Sent => RenewalPlayerOutcome::Queued,
        Submission::Nothing => RenewalPlayerOutcome::Unresolved,
    }
}

/// The `RenewalPlayerStep` a `front` step takes: `Request` (request, or
/// retry after a retryable closure), `Accept` or `Decline`.
pub(crate) const fn front_player_step(step: FrontStep) -> RenewalPlayerStep {
    match step {
        FrontStep::Request => RenewalPlayerStep::Request,
        FrontStep::Accept(_) => RenewalPlayerStep::Accept,
        FrontStep::Decline(_) => RenewalPlayerStep::Decline,
    }
}

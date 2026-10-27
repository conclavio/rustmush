//! 17g C2d (C2 plan §2.1's renewal exception, §2.6): the renewal services'
//! player steps as one public front. `@mindrenewal` and `@housemindrenewal`
//! keep their own switches and print exactly their pre-C2d sentences; TacSim's
//! `+minds/renew` calls the same steps through each service's `player_step`.
//! Both spellings reach one surface underneath: the service's own admission,
//! retained entry, idempotency key and continuation, which still grants or
//! refunds a funded offer by itself. Nothing here adds an authority: the step
//! acts only on the calling actor's own licence (the renewal reader's own
//! holder check), and every refusal is the service's one uniform line.
//!
//! The House service shares these types (its shapes are equal).

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
    /// One submission was made (or the actor's earlier one is still in the
    /// driver's queue, as `@…renewal` has always reported it).
    Queued,
    /// The service's retained entry for this selection still awaits its
    /// earlier submission's terminal; nothing was submitted.
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

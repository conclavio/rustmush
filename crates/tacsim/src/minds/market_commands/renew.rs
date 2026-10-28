//! 17g C2d (C2 plan §2.1's renewal exception, §2.6): `+minds/renew
//! <licence>` and its `/pay` and `/decline` steps, a front over the existing
//! renewal services. The licence must be a paid licence the actor holds now
//! (Roll's own-only `OwnPaid`, folded into the original-holdings read), and
//! its House flag picks the service: `mush_market::mind_renewal` for a
//! personal licence, `mush_market::mind_house_renewal` for a House Pin. Each
//! step is that service's own player step, the same code `@mindrenewal` and
//! `@housemindrenewal` run: its admission, retained entry, idempotency key
//! and submission. Nothing renews by itself (D-UM-39): the holder asks for an
//! offer, sees its frozen price, and pays it explicitly; the service then
//! grants or refunds the funded offer on its own, as it always has.
//!
//! **The hand-off (17g C2h, the lead's ruling R3, the C2e plan §5.1).**
//! `+minds` is TacSim's command, so its context reads as TacSim, while each
//! renewal service reads through the market's own renewal read and typed
//! families, which mush-core refuses to a TacSim context. So TacSim does not
//! take the step: each service registers a `front` continuation and, where
//! the player market composes, delegates it to TacSim at composition
//! (`MindCommerce::configure_market`, `mush_core::Registrar::
//! delegate_continuation`). This command checks TacSim's own refusals,
//! synchronously and in this order, then schedules the service's front for
//! the actor with `mush_market::mind_renewal::player::front_argument`:
//! 1. the licence number's shape (the `+minds` help otherwise);
//! 2. a paid licence the actor holds (`NO_LICENCE`, for a foreign licence and
//!    a missing one alike);
//! 3. `/pay` and `/decline` only: a drafted offer waiting on it
//!    (`NO_RENEWAL`), whose pending operation (`PaidHoldingView::
//!    renewal_operation`) the step names, so the payment answers the offer
//!    the holder saw and a changed offer is the service's refusal;
//! 4. the service's front, composed, enabled and delegated (`NOT_OPEN`);
//! 5. a live, connected player with an account (`NOT_OPEN`), with no
//!    submission awaiting its receipt and room in the queue (`BUSY`);
//! 6. the continuation itself (`NOT_OPEN` if mush-core refuses it: a line
//!    queued by `@wait` dispatches with no connection, which the delegation
//!    does not admit).
//!
//! On success this command prints nothing. The service's own handler then
//! runs, under the market's identity, for the same actor, on the world
//! loop's next timer wake (a 0 ms delay, provisional): it reads the licence
//! through its own holder check, takes the step, and only then prints the
//! `+minds` words (`RENEW_NOT_DUE`, `RENEW_OFFER` and `RENEW_HOW`,
//! `RENEW_FUNDED`, `RENEWING`, `RENEW_PAYING`, `RENEW_DECLINING`, now the
//! market's `mind_renewal::player` words) or its one uniform refusal
//! (`Renewal is unavailable or has changed.`), so no request line ever
//! promises a submission that was not made. The engine's `Queued.` and the
//! driver's line follow a submission as for every command. The scheduler is
//! RAM-only: a restart before the continuation runs drops it, and nothing
//! was printed or submitted.

use mush_core::{CmdError, Ctx};
use mush_market::mind_renewal::player::{FrontStep, front_argument};
use mush_roll::commerce::LicenceId;
use mush_roll::commerce::command_reader::{
    PaidHoldingView, RenewalMark, RollCommandReadRequest, RollCommandReadResult,
};
use mush_roll::commerce::holding_reader::{OriginalReadRequest, OriginalReadResult};

use super::words::{self, Outcome, Refusal, fill, word};

/// The continuation's delay, in milliseconds (17g §1 ruling 6:
/// provisional): the service's handler runs on the next timer wake.
const FRONT_DELAY_MS: i64 = 0;

/// Which `+minds/renew` form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Renew {
    Status,
    Pay,
    Decline,
}

/// 17g C3a: what a renewal form selects: the licence number and the form
/// (`parse_renew`; the typed `Renew`, `RenewPay` and `RenewDecline`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct RenewSelection {
    pub(super) licence: u64,
    pub(super) which: Renew,
}

fn not_open() -> Refusal {
    Refusal::not_open()
}

/// 17g C2d: a paid licence the actor holds now (`OwnPaid`), or
/// `NO_LICENCE`: another character's licence reads exactly like a missing
/// one. `NOT_OPEN` without the command slot.
pub(super) fn own_paid(ctx: &Ctx<'_>, licence: u64) -> Result<PaidHoldingView, Refusal> {
    let refuse = || word!(NO_LICENCE, &[("licence", &licence.to_string())]);
    let id = LicenceId::new(licence).map_err(|_| refuse())?;
    let service = super::service(ctx)?;
    let request = OriginalReadRequest::command(RollCommandReadRequest::own_paid(id));
    ctx.read_owner(&service.roll, request)
        .ok()
        .and_then(OriginalReadResult::into_command)
        .and_then(RollCommandReadResult::into_own_paid)
        .ok_or_else(refuse)
}

/// The step a form hands to the service: `+minds/renew` asks (or shows the
/// offer that exists); `/pay` and `/decline` act on the drafted offer the
/// holder's own paid holding names, else `NO_RENEWAL`.
fn front_step(held: &PaidHoldingView, which: Renew) -> Result<FrontStep, Refusal> {
    let offered = match (held.renewal(), held.renewal_operation()) {
        (Some(RenewalMark::Offered), Some(operation)) => Some(operation),
        _ => None,
    };
    let no_renewal = || {
        word!(
            NO_RENEWAL,
            &[("licence", &held.licence().get().to_string())]
        )
    };
    Ok(match which {
        Renew::Status => FrontStep::Request,
        Renew::Pay => FrontStep::Accept(offered.ok_or_else(no_renewal)?),
        Renew::Decline => FrontStep::Decline(offered.ok_or_else(no_renewal)?),
    })
}

/// `+minds/renew <licence>`, `/pay <licence>` or `/decline <licence>`: the
/// synchronous refusals of the module doc, then the hand-off to the
/// service's front. Prints nothing on success: 17g C3a's `run_renew`
/// answers `Outcome::Staged(RENEW_HANDED)` for the typed reply, and this
/// line command DISCARDS it, so the line stays as silent as 17g C2h built it
/// (the one place the line and the typed reply differ, the C3 plan's
/// departure 7).
pub(crate) fn renew(ctx: &mut Ctx<'_>, text: &str, which: Renew) -> Result<(), CmdError> {
    let selection = parse_renew(text, which)?;
    run_renew(ctx, selection)?;
    Ok(())
}

/// 17g C3a: a renewal form's argument, the licence number (`number`), else
/// the `+minds` help (`REFUSED`).
pub(super) fn parse_renew(text: &str, which: Renew) -> Result<RenewSelection, Refusal> {
    let licence = super::author::number(text).ok_or_else(|| Refusal::usage(super::market::HELP))?;
    Ok(RenewSelection { licence, which })
}

/// 17g C3a: `renew` after its parse, for the line and the typed renewal
/// steps alike: the synchronous refusals of the module doc, the hand-off,
/// and then `Outcome::Staged` with `RENEW_HANDED` for the licence, which
/// only the typed reply carries (`renew` discards it).
pub(super) fn run_renew(ctx: &mut Ctx<'_>, selection: RenewSelection) -> Result<Outcome, Refusal> {
    let held = own_paid(ctx, selection.licence)?;
    let step = front_step(&held, selection.which)?;
    let front = if held.house() {
        mush_market::mind_house_renewal::player_front(ctx)
    } else {
        mush_market::mind_renewal::player_front(ctx)
    }
    .ok_or_else(not_open)?;
    let actor = super::principal(ctx).ok_or_else(not_open)?;
    if ctx.authoritative_pending(actor) || ctx.authoritative_queue_full() {
        return Err(word!(BUSY));
    }
    let argument = front_argument(held.licence(), step);
    ctx.schedule_continuation(&front, actor, &argument, FRONT_DELAY_MS)
        .map_err(|_| not_open())?;
    Ok(Outcome::Staged(fill(
        words::RENEW_HANDED,
        &[("licence", &held.licence().get().to_string())],
    )))
}

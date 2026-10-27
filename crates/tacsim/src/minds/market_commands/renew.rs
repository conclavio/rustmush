//! 17g C2d (C2 plan §2.1's renewal exception, §2.6): `+minds/renew
//! <licence>` and its `/pay` and `/decline` steps, a front over the existing
//! renewal services. The licence must be a paid licence the actor holds now
//! (Roll's own-only `OwnPaid`, folded into the original-holdings read), and
//! its House flag picks the service: `mush_market::mind_renewal` for a
//! personal licence, `mush_market::mind_house_renewal` for a House Pin. Each
//! step is that service's own `player_step`, the same code `@mindrenewal` and
//! `@housemindrenewal` run: its admission, retained entry, idempotency key
//! and submission. Nothing renews by itself (D-UM-39): the holder asks for an
//! offer, sees its frozen price, and pays it explicitly; the service then
//! grants or refunds the funded offer on its own, as it always has. A
//! refusal from the service is its own uniform line (`Renewal is unavailable
//! or has changed.`), for a foreign licence and a missing one alike.
//!
//! - `+minds/renew <licence>`: with no offer, or after the last one closed or
//!   granted, `RENEW_NOT_DUE` while the term runs (the service's own due
//!   rule, `world_ic_now().at < term_end`), else ask for one (`RENEWING`);
//!   a drafted offer prints `RENEW_OFFER` and `RENEW_HOW` and submits
//!   nothing; a funded one `RENEW_FUNDED`.
//! - `/pay` and `/decline` need a drafted offer (`NO_RENEWAL`), then submit
//!   the service's accept or decline (`RENEW_PAYING` repeats the frozen
//!   price; `RENEW_DECLINING`).
//!
//! `RENEWING`, `RENEW_PAYING` and `RENEW_DECLINING` are printed once the
//! service has queued the submission, so a step the service refuses prints
//! only its refusal; the engine's `Queued.` and the driver's line follow.
//!
//! **Held (the C2d fix round, reviewers C2D-R1, F1 and T1).** `player_step`
//! cannot answer from this command: `+minds` is TacSim's command, so its
//! context reads as TacSim (`ModuleActionState::for_dispatch` names the
//! registering module), while each renewal service reads through the
//! market's own owner-read handle (`RenewalReader`, requested by and allowed
//! only to `market`) and the market's own typed families. mush-core refuses
//! both reads to a TacSim context (`OwnerReadError::Authority`; a foreign
//! family's `require_owner`), and the service maps every refusal to its one
//! uniform line, so every held licence would read `Renewal is unavailable or
//! has changed.` The owner-read caps were full when this was held (32/32,
//! 17g C2a; 48/48 since 17g C2e), so TacSim could not take a renewal read of
//! its own either. A seam that runs the step
//! under the market's identity is the lead's ruling. Until it lands, all
//! three forms check their shape and the actor's own licence (`NO_LICENCE`),
//! then refuse `NOT_OPEN` at `RENEWALS_OPEN`, by C2c's rule for a command
//! that cannot complete (reviewer C2C-3); the rest of `renew` is built and
//! waits for that one constant, and its first dispatched test (a held
//! licence reaching `RENEW_NOT_DUE`, story part 3) waits with it.

use mush_core::{CmdError, Ctx};
use mush_market::mind_renewal::{
    RenewalPlayerOutcome, RenewalPlayerStep, RenewalState, RenewalStatusView,
};
use mush_roll::commerce::LicenceId;
use mush_roll::commerce::command_reader::{
    PaidHoldingView, RollCommandReadRequest, RollCommandReadResult,
};
use mush_roll::commerce::holding_reader::{OriginalReadRequest, OriginalReadResult};

use super::market::days;
use super::words::{self, fill};

const HOUR: i64 = 3_600;

/// Whether `+minds/renew` may call the renewal services' `player_step`:
/// false until a seam runs the step under the market's identity (module
/// doc). All three forms refuse `NOT_OPEN` after `own_paid` while it is
/// false.
pub(crate) const RENEWALS_OPEN: bool = false;

/// Which `+minds/renew` form.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Renew {
    Status,
    Pay,
    Decline,
}

fn not_open() -> CmdError {
    CmdError::msg(words::NOT_OPEN)
}

/// 17g C2d: a paid licence the actor holds now (`OwnPaid`), or
/// `NO_LICENCE`: another character's licence reads exactly like a missing
/// one. `NOT_OPEN` without the command slot.
pub(super) fn own_paid(ctx: &Ctx<'_>, licence: u64) -> Result<PaidHoldingView, CmdError> {
    let refuse = || {
        CmdError::msg(fill(
            words::NO_LICENCE,
            &[("licence", &licence.to_string())],
        ))
    };
    let id = LicenceId::new(licence).map_err(|_| refuse())?;
    let service = super::service(ctx)?;
    let request = OriginalReadRequest::command(RollCommandReadRequest::own_paid(id));
    ctx.read_owner(&service.roll, request)
        .ok()
        .and_then(OriginalReadResult::into_command)
        .and_then(RollCommandReadResult::into_own_paid)
        .ok_or_else(refuse)
}

/// One step on the service the licence's House flag selects.
fn step(
    ctx: &mut Ctx<'_>,
    house: bool,
    licence: LicenceId,
    step: RenewalPlayerStep,
) -> Result<RenewalPlayerOutcome, CmdError> {
    if house {
        mush_market::mind_house_renewal::player_step(ctx, licence, step)
    } else {
        mush_market::mind_renewal::player_step(ctx, licence, step)
    }
}

/// The licence's renewal status, through the service's own status branch.
fn status(
    ctx: &mut Ctx<'_>,
    house: bool,
    licence: LicenceId,
) -> Result<RenewalStatusView, CmdError> {
    match step(ctx, house, licence, RenewalPlayerStep::Status)? {
        RenewalPlayerOutcome::Status(view) => Ok(view),
        // The status branch submits nothing.
        RenewalPlayerOutcome::Queued | RenewalPlayerOutcome::Unresolved => Err(not_open()),
    }
}

/// A submitting step (C2 plan §2.1 steps 1 and 2: a live, connected player
/// with an account; `BUSY` while a submission awaits its receipt), then the
/// request line once the service queued it. The service's `Unresolved` (its
/// retained entry still awaits an earlier terminal) is `BUSY`.
fn submit_step(
    ctx: &mut Ctx<'_>,
    house: bool,
    licence: LicenceId,
    kind: RenewalPlayerStep,
    request: &str,
) -> Result<(), CmdError> {
    let actor = super::principal(ctx).ok_or_else(not_open)?;
    if ctx.authoritative_pending(actor) || ctx.authoritative_queue_full() {
        return Err(CmdError::msg(words::BUSY));
    }
    match step(ctx, house, licence, kind)? {
        RenewalPlayerOutcome::Queued => {
            ctx.out.info(actor, request.to_owned());
            Ok(())
        }
        RenewalPlayerOutcome::Unresolved | RenewalPlayerOutcome::Status(_) => {
            Err(CmdError::msg(words::BUSY))
        }
    }
}

/// `+minds/renew <licence>`, `/pay <licence>` or `/decline <licence>`.
pub(crate) fn renew(ctx: &mut Ctx<'_>, text: &str, which: Renew) -> Result<(), CmdError> {
    let number = super::author::number(text).ok_or_else(|| CmdError::msg(super::market::HELP))?;
    let held = own_paid(ctx, number)?;
    if !RENEWALS_OPEN {
        return Err(not_open());
    }
    let licence = held.licence();
    let house = held.house();
    let view = status(ctx, house, licence)?;
    let shown = number.to_string();
    let actor = ctx.actor;
    match which {
        Renew::Status => match view.state() {
            RenewalState::Drafted => {
                let now = ctx.w.world_ic_now().at;
                for line in offer_lines(&shown, &view, now) {
                    ctx.out.info(actor, line);
                }
                Ok(())
            }
            RenewalState::Funded => {
                ctx.out
                    .info(actor, fill(words::RENEW_FUNDED, &[("licence", &shown)]));
                Ok(())
            }
            RenewalState::None | RenewalState::Closed | RenewalState::Granted => {
                let now = ctx.w.world_ic_now().at;
                if now < view.term_end_ic() {
                    let left = days(view.term_end_ic().saturating_sub(now)).to_string();
                    return Err(CmdError::msg(fill(
                        words::RENEW_NOT_DUE,
                        &[("licence", &shown), ("days", &left)],
                    )));
                }
                let line = fill(words::RENEWING, &[("licence", &shown)]);
                submit_step(ctx, house, licence, RenewalPlayerStep::Request, &line)
            }
        },
        Renew::Pay | Renew::Decline => {
            if view.state() != RenewalState::Drafted {
                return Err(CmdError::msg(fill(
                    words::NO_RENEWAL,
                    &[("licence", &shown)],
                )));
            }
            let (kind, line) = if which == Renew::Pay {
                let price = crate::launch::credits(view.price_minor().unwrap_or_default());
                (
                    RenewalPlayerStep::Accept,
                    fill(
                        words::RENEW_PAYING,
                        &[("price", &price), ("licence", &shown)],
                    ),
                )
            } else {
                (
                    RenewalPlayerStep::Decline,
                    fill(words::RENEW_DECLINING, &[("licence", &shown)]),
                )
            };
            submit_step(ctx, house, licence, kind, &line)
        }
    }
}

/// A drafted offer's two lines: `RENEW_OFFER` (the frozen price, the renewed
/// term and Grace in whole IC days, the IC hours left, rounded up, never
/// below 0) and `RENEW_HOW`.
pub(crate) fn offer_lines(licence: &str, view: &RenewalStatusView, now_ic: i64) -> [String; 2] {
    offer_text(
        licence,
        view.price_minor().unwrap_or_default(),
        view.term_seconds().unwrap_or_default(),
        view.grace_seconds().unwrap_or_default(),
        view.offer_expires_ic().unwrap_or(now_ic),
        now_ic,
    )
}

/// `offer_lines` from the view's raw fields (the frozen price in minor
/// units, the renewed term and Grace in IC seconds, the offer's IC expiry),
/// so the sentences are tested without a renewal service.
pub(crate) fn offer_text(
    licence: &str,
    price_minor: i64,
    term_seconds: i64,
    grace_seconds: i64,
    expires_ic: i64,
    now_ic: i64,
) -> [String; 2] {
    let price = crate::launch::credits(price_minor);
    let term = days(term_seconds).to_string();
    let grace = days(grace_seconds).to_string();
    let hours = hours_left(expires_ic, now_ic).to_string();
    [
        fill(
            words::RENEW_OFFER,
            &[
                ("licence", licence),
                ("price", &price),
                ("term", &term),
                ("grace", &grace),
                ("hours", &hours),
            ],
        ),
        fill(words::RENEW_HOW, &[("licence", licence)]),
    ]
}

/// Whole hours from `now` to `until`, rounded up, never below 0.
pub(crate) fn hours_left(until: i64, now: i64) -> i64 {
    until.saturating_sub(now).max(0).saturating_add(HOUR - 1) / HOUR
}

//! 17g C2 (the owner's approved command table, 17g §1 ruling 4): line-mode
//! commands over the composed mind commerce. Each command wraps exactly one
//! existing legal action (C2 plan §2.1):
//!
//! 1. the actor is a live, connected, non-GOING player with an account (the
//!    workshop's `WorkshopCommands::principal` rule);
//! 2. a command already awaiting its receipt, or a full queue, refuses `BUSY`;
//! 3. it reads only the actor's own committed state (its library, its bulk
//!    account's Registry runs, the bounded Roll command kinds folded into the
//!    original-holdings read, its own personal account) and public state;
//! 4. it builds the wrapped action's parameters from what it read; the player
//!    supplies only selections (a library reference, a filed or licence
//!    number, a name);
//! 5. it prints one request line, never a success line, and calls
//!    `Ctx::submit_authoritative` exactly once.
//!
//! The server's `command_offer_submission` then regenerates this actor's own
//! projected offers and resolves the matching one, the provider rechecks every
//! authority at preparation, a refusal reaches the actor as the driver's one
//! line and success as the provider's commit-bound sentence. So no command
//! adds authority, and an invocation-side refusal is byte-identical whatever
//! its reason.
//!
//! C2a builds the mechanism and the author's commands (`author`); C2c adds
//! `market` (`+minds`, `+minds/buy`, `@mind/publish`) and `holdings`
//! (`+minds/holdings`); C2d adds `renew` (`+minds/renew`, a front over the
//! renewal services' `player_step`, the one command that submits through a
//! service rather than `submit`; held at `renew::RENEWALS_OPEN`), `trade`
//! (`+minds/resell`, `+minds/charter` and their `/accept`s; held at
//! `trade::NAMED_OFFERS_OPEN`) and `registry` (`@registry/recall` and
//! `@registry/revoke`).

pub(crate) mod author;
pub(crate) mod holdings;
pub(crate) mod market;
pub(crate) mod registry;
pub(crate) mod renew;
pub(crate) mod trade;
pub(crate) mod words;

use std::sync::{Arc, Mutex};

use mush_core::action::{IdempotencyKeyText, PerceivedRef};
use mush_core::read_service::OwnerReadHandle;
use mush_core::{CmdError, Ctx, DefinitionRef, Flags, ModError, ObjId, ObjKind, Registrar};
use mush_market::mind_commands::MarketCommandReader;
use mush_roll::commerce::holding_reader::OriginalOwnerReader;

/// 17g C2a: the command adapter's own state slot (tacsim's, no state-slot
/// row): the granted Roll read and the per-IC-second ordinal of the
/// idempotency key. `roll` is TacSim's one original-holdings grant, shared
/// with the holdings seam (`match_::original_access::register_entitlements`):
/// the command kinds are folded into that read (C2 plan §2.4 fallback 1),
/// because the format-2 Active launch had no owner-read grant to spare at
/// 17g C2a. It is registered only when Active commerce composes the player
/// market (`MindCommerce::configure_tacsim` sets
/// `TacsimRuleset.mind_commands`), so on a House-only launch every command
/// that needs it refuses `NOT_OPEN`. A slot is `Clone` (`Registrar::state`);
/// a clone shares the one counter.
///
/// 17g C2c: `market` is the market's bounded command read
/// (`mush_market::mind_commands`), which every `+minds` form and
/// `@mind/publish` need. 17g C2e (the lead's ruling R1 raised the owner-read
/// caps to 48): `TacsimRuleset::register` requests it with
/// `MarketCommandReader::request` over the same composed manifest the
/// entitlement holdings' grants take, in the branch that registers this slot,
/// so it is `Some` exactly where the slot exists (the format-2 Active launch:
/// 32 services and 33 grants). The `Option` stays so that every form keeps
/// its `NOT_OPEN` refusal for a composition without it.
#[derive(Clone)]
pub(crate) struct MindCommandService {
    pub(crate) roll: OwnerReadHandle<OriginalOwnerReader>,
    pub(crate) market: Option<OwnerReadHandle<MarketCommandReader>>,
    /// `(IC second, next ordinal)`: RAM only, so a restart starts again at 0,
    /// and keys stay distinct because each carries its IC second.
    attempts: Arc<Mutex<(i64, u64)>>,
}

/// Register the slot over the original-holdings grant the holdings seam
/// already requested with the same composed manifest (no second Roll grant),
/// and the market's command read TacSim requested beside it (17g C2e).
pub(crate) fn register(
    reg: &mut Registrar<'_>,
    roll: OwnerReadHandle<OriginalOwnerReader>,
    market: Option<OwnerReadHandle<MarketCommandReader>>,
) -> Result<(), ModError> {
    reg.state(MindCommandService {
        roll,
        market,
        attempts: Arc::new(Mutex::new((i64::MIN, 0))),
    });
    Ok(())
}

/// The slot, when this crate owns it (Active with the player market).
pub(crate) fn service<'a>(ctx: &'a Ctx<'_>) -> Result<&'a MindCommandService, CmdError> {
    if ctx
        .w
        .module_state_owner(std::any::TypeId::of::<MindCommandService>())
        .map(|owner| owner.as_str())
        != Some(crate::NS)
    {
        return Err(CmdError::msg(words::NOT_OPEN));
    }
    ctx.w
        .module_state::<MindCommandService>()
        .ok_or_else(|| CmdError::msg(words::NOT_OPEN))
}

/// The next idempotency key of `verb` on `subject` at IC second `at`:
/// `mind.<verb>.<subject>.<at:x>.<ordinal:x>`, the industry idiom
/// (`mush_industry::commands::submit`). The ordinal restarts at 0 in each new
/// IC second and counts up within one, so a deliberately retyped command is a
/// new request while the driver retrying one submission is still one commit.
/// A key outside the key grammar refuses `NOT_OPEN`.
pub(crate) fn next_key(
    attempts: &Mutex<(i64, u64)>,
    verb: &str,
    subject: &str,
    at: i64,
) -> Result<IdempotencyKeyText, CmdError> {
    let not_open = || CmdError::msg(words::NOT_OPEN);
    let ordinal = {
        let mut state = attempts.lock().map_err(|_| not_open())?;
        if state.0 != at {
            *state = (at, 0);
        }
        let ordinal = state.1;
        state.1 = ordinal.checked_add(1).ok_or_else(not_open)?;
        ordinal
    };
    IdempotencyKeyText::parse_canonical(format!("mind.{verb}.{subject}.{at:x}.{ordinal:x}"))
        .map_err(|_| not_open())
}

/// The actor, when it is a live, connected, non-GOING player with an account.
fn principal(ctx: &Ctx<'_>) -> Option<ObjId> {
    let actor = ctx.actor;
    let live = ctx
        .w
        .get(actor)
        .is_some_and(|o| o.kind == ObjKind::Player && !o.flags.contains(Flags::GOING));
    let account = ctx
        .w
        .module_state::<mush_sim::AccountOracle>()
        .and_then(|oracle| oracle.account_of(actor))
        .is_some();
    (live && account && ctx.w.has_live_conns(actor)).then_some(actor)
}

/// 17g C2a (C2 plan §2.1 steps 1, 2 and 5): submit one existing legal action
/// for the actor with server-built `parameters`. `request` is the request
/// line, printed to the actor alone once every check has passed and just
/// before the one `Ctx::submit_authoritative`; the key's discriminator is
/// `mind.<verb>.<subject>`. It writes nothing else: the mechanic is the
/// provider's and the confirmation the driver's, after the durable receipt.
pub(crate) fn submit(
    ctx: &mut Ctx<'_>,
    verb: &str,
    subject: &str,
    action: DefinitionRef,
    parameters: serde_json::Value,
    request: &str,
) -> Result<(), CmdError> {
    let actor = principal(ctx).ok_or_else(|| CmdError::msg(words::NOT_OPEN))?;
    if ctx.authoritative_pending(actor) || ctx.authoritative_queue_full() {
        return Err(CmdError::msg(words::BUSY));
    }
    let at = ctx.w.world_ic_now().at;
    let key = next_key(&service(ctx)?.attempts, verb, subject, at)?;
    ctx.out.info(actor, request.to_owned());
    ctx.submit_authoritative(action, PerceivedRef::SelfActor, Vec::new(), parameters, key);
    Ok(())
}

/// 17g C2a (C2 plan §7): the actor's one personal credit account, the payer
/// of every command that pays: the unique Active, unscoped `tacsim.credit`
/// account owned by the actor itself (`mind_entitlement::accounts::personal`'s
/// rule). None or several refuse `ACCOUNT`; buyer choice stays on `+act`.
pub(crate) fn personal_account(ctx: &Ctx<'_>) -> Result<mush_economy::AccountId, CmdError> {
    let refuse = || CmdError::msg(words::ACCOUNT);
    let economy = ctx
        .w
        .module_state::<mush_economy::EconomyStore>()
        .ok_or_else(refuse)?;
    if economy.is_frozen() {
        return Err(refuse());
    }
    let instrument = mush_economy::InstrumentId::new("tacsim.credit").ok_or_else(refuse)?;
    let snapshot = economy.snapshot();
    let owner = mush_org::PrincipalId::Object(ctx.actor);
    let mut accounts = snapshot.accounts_of(&owner).filter(|account| {
        account.instrument == instrument
            && account.scope.is_none()
            && account.status == mush_economy::AccountStatus::Active
    });
    match (accounts.next(), accounts.next()) {
        (Some(account), None) => Ok(account.id),
        _ => Err(refuse()),
    }
}

// 17g C2c (C2a's residual): declared last, so a first-literal source census
// reads every line above as production code.
#[cfg(test)]
mod tests;

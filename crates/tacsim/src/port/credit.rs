//! **`+credit`** (doc 18 cut IC3; doc 18 §4.1; lead ruling L12): the pilot's
//! wallet, a port of the retired `+nuyen`.
//!
//! A pure read of the actor's **own** unscoped `tacsim.credit` account in the
//! economy snapshot — posted, held and available, each through the economy's one
//! amount render (`7.00cr`). It takes no argument, so it reads no other account
//! and is no existence oracle: staff read anyone's through `@account/show`.
//! There is no "owed" column: under R2 a captain carries no debt (doc 18 §4.1).
//! It reads anywhere — Limbo, a station, a hull.

use mush_core::{Ctx, ObjId, World};
use mush_economy::{Account, AccountStatus, EconomyStore, InstrumentId};
use mush_org::PrincipalId;

use crate::economy::CREDIT;

/// The verb.
pub const CREDIT_VERB: &str = "+credit";

/// `+credit`'s help.
pub const CREDIT_HELP: &str = "+credit  --  Your credit account: what is posted to it, what is \
     held against purchases you have accepted, and what you can spend now, in credits. It reads \
     anywhere. An account is opened in your name when you enrol with a backer at the Registry & \
     Co-op desk.";

/// No account in the actor's name.
pub const NO_ACCOUNT: &str = "No credit account is open in your name.";

/// An account whose available balance overflows.
pub const UNREADABLE: &str = "Your account's balance cannot be read right now.";

/// **The one `+credit` line**, from the three amounts already rendered.
#[must_use]
pub fn credit_line(posted: &str, held: &str, available: &str) -> String {
    format!("Credit: {posted} posted, {held} held, {available} available.")
}

/// **The actor's own credit account**: Active, owned by the actor as an object
/// principal, in `tacsim.credit`, unscoped; the lowest id if several.
#[must_use]
pub fn own_account(w: &World, actor: ObjId) -> Option<&Account> {
    let instrument = InstrumentId::new(CREDIT)?;
    let owner = PrincipalId::Object(actor);
    w.module_state::<EconomyStore>()?
        .snapshot()
        .accounts_of(&owner)
        .filter(|a| {
            a.status == AccountStatus::Active && a.instrument == instrument && a.scope.is_none()
        })
        .min_by_key(|a| a.id)
}

/// The handler.
///
/// # Errors
///
/// The disabled-layer refusal.
pub(crate) fn credit_cmd(ctx: &mut Ctx<'_>) -> Result<(), mush_core::CmdError> {
    let enabled = ctx
        .w
        .module_state::<crate::store::TacsimStore>()
        .is_some_and(crate::store::TacsimStore::enabled);
    if !enabled {
        return Err(mush_core::CmdError::msg(crate::commands::NOT_ENABLED));
    }
    let actor = ctx.actor;
    let line = match own_account(ctx.w, actor) {
        None => NO_ACCOUNT.to_owned(),
        Some(account) => match (account.available(), InstrumentId::new(CREDIT)) {
            (Some(available), Some(instrument)) => credit_line(
                &mush_economy::money(ctx, &instrument, account.posted_balance),
                &mush_economy::money(ctx, &instrument, account.held_amount),
                &mush_economy::money(ctx, &instrument, available),
            ),
            _ => {
                tracing::error!(
                    account = account.id.0,
                    "tacsim: +credit could not read an account's available balance"
                );
                UNREADABLE.to_owned()
            }
        },
    };
    ctx.out.sys_to_actor(actor, "tacsim", line);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ic3_the_credit_line_names_all_three() {
        assert_eq!(
            credit_line("25000.00cr", "1200.50cr", "23799.50cr"),
            "Credit: 25000.00cr posted, 1200.50cr held, 23799.50cr available."
        );
    }
}

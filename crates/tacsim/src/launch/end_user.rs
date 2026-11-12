//! **`+enduser`: the Registry's Class III end-user registration** (doc 44 unit MW.23,
//! Q-44-40; doc 17 §16; 17f M7.11).
//!
//! A Class III core is built for, and handed to, only a principal who holds the end-user
//! entitlement (`tacsim.licence.core3_end_user`); the economy checks it at the integration's
//! acceptance and completion and at the delivery. Before this unit only a test could issue it,
//! so no launch world could hold a Class III core.
//!
//! The route is the kind every other right of a launch is issued by: **the end user's own
//! paid act, the fee into the launch's Registry account, with no clerk and no staff on the
//! path** (a filing, a Pin, and the prospecting licence's `+licence/buy` are all that). A
//! character types `+enduser/apply` (a preview, which writes nothing) and `+enduser/apply go`.
//! In one transaction the fee moves from their own credit account to the Registry's
//! (`LaunchDeployment::registry`, where the filing fee and the levy go) and one unit of the
//! entitlement is issued to them by the launch's `registry_issuer`, the system principal that
//! already issues its charter and core titles. The economy owns that transaction
//! (`mush_economy::core_integration::register_end_user`); this file decides who may ask and
//! what it costs.
//!
//! One registration a character. It has no term: the entitlement is an account balance the
//! economy reads at the act, and a lapse would need a record and a gate this unit does not
//! build. The Registry suspends an end user by freezing the account, which refuses them both a
//! Class III core and a second registration.
//!
//! The route exists only on an **Active** launch: with no launch, or one still at its
//! `Initialize` boot, the verb says the Registry takes no registrations. It needs nothing in
//! the launch's deployment file and nothing from the bring-up: the first registration on a
//! world registers the instrument and opens the issuer's source account.

use mush_core::{CmdError, Ctx, World};
use mush_economy::AccountId;
use mush_economy::core_integration::{
    EndUserError, EndUserRegistration, EndUserStanding, check_end_user_registration,
    end_user_standing, register_end_user,
};
use mush_org::SystemPrincipalId;

use super::LaunchMode;
use crate::commands::NOT_ENABLED;
use crate::port::coop::{money_minor, say};
use crate::store::TacsimStore;

/// The verb.
pub const END_USER_VERB: &str = "+enduser";
/// **The registration's fee**, centicredits: 2,500cr, a tenth of the Class III core it admits
/// (`CoreClass::III`'s 25,000cr) and the prospecting licence's fee. Provisional, like every
/// economy number (the owner's rule): tuned in play.
pub const END_USER_FEE_MINOR: i64 = 250_000;

/// `+enduser`'s help.
pub const END_USER_HELP: &str = "+enduser  --  The Registry's Class III end-user registration: \
     without it no shop builds you a Class III core or hands one over. +enduser (or \
     +enduser/show) reads where you stand, from anywhere.\n+enduser/apply  previews a \
     registration: 2500.00cr, once, paid to the Registry from your own credit account. \
     +enduser/apply go registers you. One registration a character; it does not lapse and \
     cannot be sold or handed on.";
/// No Active launch on this world.
pub const END_USER_NOT_OPEN: &str = "The Registry takes no end-user registrations on this world.";
/// Not registered.
pub const END_USER_UNLISTED: &str = "The Registry does not list you as a Class III end user. \
     +enduser/apply registers you: {fee}, once.";
/// Registered.
pub const END_USER_LISTED: &str = "The Registry lists you as a Class III end user, account \
     #{account}. A shop may build you a Class III core and hand it over.";
/// The entitlement account is not `Active`.
pub const END_USER_SUSPENDED: &str = "The Registry has suspended your Class III end-user \
     registration, account #{account}. Ask staff about it.";
/// No credit account.
pub const END_USER_NO_CREDIT: &str = "You hold no credit account to pay the Registry from.";
/// Short of the fee.
pub const END_USER_FUNDS: &str = "An end-user registration costs {fee}; you have {available}.";
/// The preview.
pub const END_USER_PREVIEW: &str = "The Registry registers you as a Class III end user for \
     {fee}, once. +enduser/apply go registers you.";
/// Registered now.
pub const END_USER_DONE: &str =
    "The Registry lists you as a Class III end user: {fee} paid, account #{account}.";
/// The registration could not be made; nothing was paid.
pub const END_USER_FAILED: &str = "The Registry could not register you; nothing was paid.";
/// The grammar.
pub const END_USER_USAGE: &str = "Usage: +enduser[/show]  or  +enduser/apply [go]";

/// What an Active launch gives the route: who issues, and where the fee goes.
struct Route {
    issuer: SystemPrincipalId,
    registry: AccountId,
}

/// **The launch's route**: `Some` only where this crate's launch is composed and Active.
fn route(world: &World) -> Option<Route> {
    if world
        .module_state_owner(std::any::TypeId::of::<super::registration::LaunchService>())
        .map(|owner| owner.as_str())
        != Some(crate::NS)
    {
        return None;
    }
    let deployment = world
        .module_state::<super::registration::LaunchService>()?
        .0
        .deployment();
    (deployment.mode == LaunchMode::Active).then(|| Route {
        issuer: deployment.registry_issuer.clone(),
        registry: deployment.registry.account,
    })
}

/// The line a standing reads as.
fn standing_line(ctx: &Ctx<'_>, standing: EndUserStanding) -> String {
    match standing {
        EndUserStanding::Unlisted => {
            END_USER_UNLISTED.replace("{fee}", &money_minor(ctx, END_USER_FEE_MINOR))
        }
        EndUserStanding::Registered { account } => {
            END_USER_LISTED.replace("{account}", &account.0.to_string())
        }
        EndUserStanding::Suspended { account } => {
            END_USER_SUSPENDED.replace("{account}", &account.0.to_string())
        }
    }
}

/// The actor's registration as the economy is asked for it, or the line that refuses it.
fn request(ctx: &Ctx<'_>, route: Route) -> Result<EndUserRegistration, String> {
    let payer = crate::port::credit::own_account(ctx.w, ctx.actor)
        .ok_or_else(|| END_USER_NO_CREDIT.to_owned())?
        .id;
    Ok(EndUserRegistration {
        issuer: route.issuer,
        holder: ctx.actor,
        payer,
        registry: route.registry,
        fee: mush_economy::Amount::from_minor(END_USER_FEE_MINOR),
    })
}

/// The line a refusal reads as. Staff learn the cause from the log, never the player.
fn refusal_line(ctx: &Ctx<'_>, error: &EndUserError) -> String {
    match error {
        EndUserError::Registered { account } => {
            END_USER_LISTED.replace("{account}", &account.0.to_string())
        }
        EndUserError::Suspended { account } => {
            END_USER_SUSPENDED.replace("{account}", &account.0.to_string())
        }
        EndUserError::Funds { available } => END_USER_FUNDS
            .replace("{fee}", &money_minor(ctx, END_USER_FEE_MINOR))
            .replace("{available}", &money_minor(ctx, available.minor())),
        EndUserError::Unavailable | EndUserError::Refused(_) => {
            tracing::error!(
                actor = ctx.actor.0,
                %error,
                "tacsim: +enduser/apply: the Registry could not register an end user"
            );
            END_USER_FAILED.to_owned()
        }
    }
}

/// **`+enduser`, `+enduser/show`, `+enduser/apply [go]`**. Accepted from anywhere.
///
/// # Errors
///
/// Every refusal is a [`CmdError`] line said to the actor.
pub(crate) fn end_user_cmd(ctx: &mut Ctx<'_>) -> Result<(), CmdError> {
    if !ctx
        .w
        .module_state::<TacsimStore>()
        .is_some_and(TacsimStore::enabled)
    {
        return Err(CmdError::msg(NOT_ENABLED));
    }
    let Some(route) = route(ctx.w) else {
        return Err(CmdError::msg(END_USER_NOT_OPEN));
    };
    let raw = ctx.args.raw().trim().to_owned();
    if !ctx.switches.has("apply") {
        if !raw.is_empty() {
            return Err(CmdError::msg(END_USER_USAGE));
        }
        let line = standing_line(ctx, end_user_standing(ctx.w, ctx.actor));
        say(ctx, line);
        return Ok(());
    }
    let go = match raw.as_str() {
        "" => false,
        word if word.eq_ignore_ascii_case("go") => true,
        _ => return Err(CmdError::msg(END_USER_USAGE)),
    };
    let request = request(ctx, route).map_err(CmdError::msg)?;
    let fee = money_minor(ctx, END_USER_FEE_MINOR);
    if !go {
        check_end_user_registration(ctx.w, &request)
            .map_err(|error| CmdError::msg(refusal_line(ctx, &error)))?;
        say(ctx, END_USER_PREVIEW.replace("{fee}", &fee));
        return Ok(());
    }
    // The registration is the handler's last fallible call (`mush_economy::post`'s rule).
    match register_end_user(ctx, &request) {
        Ok(registered) => {
            say(
                ctx,
                END_USER_DONE
                    .replace("{fee}", &fee)
                    .replace("{account}", &registered.account.0.to_string()),
            );
            Ok(())
        }
        Err(error) => Err(CmdError::msg(refusal_line(ctx, &error))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The help states the fee the verb charges, and it is a tenth of the Class III core.
    #[test]
    fn mw23_the_help_states_the_fee_the_verb_charges() {
        let whole = END_USER_FEE_MINOR / 100;
        assert_eq!(END_USER_FEE_MINOR % 100, 0);
        assert!(
            END_USER_HELP.contains(&format!("{whole}.00cr")),
            "{END_USER_HELP}"
        );
        assert_eq!(
            END_USER_FEE_MINOR * 10,
            mush_economy::core_integration::CoreClass::III.finished_minor()
        );
    }

    /// The entitlement the route issues is the one the hardware checks.
    #[test]
    fn mw23_the_route_issues_the_entitlement_doc_17_names() {
        assert_eq!(
            mush_economy::core_integration::CORE3_ENTITLEMENT,
            "tacsim.licence.core3_end_user"
        );
    }
}

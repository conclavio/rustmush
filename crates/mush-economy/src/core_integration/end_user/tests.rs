//! The end-user register over a real economy: the fee and the entitlement are one
//! transaction, a holder is registered once, and a restart keeps the registration.
use super::*;
use crate::EconomyModule;
use mush_core::{ArgShape, CmdError, CommandSpec, GOD, GameModule, ModError, Registrar};
use mush_testkit::{TestWorld, fold_module_ops, text_of};
use std::sync::Arc;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const ISSUER: &str = "hardware.factory";
const CREDIT: &str = "tacsim.credit";
const FEE: i64 = 250_000;

/// A stand-in for the ruleset that owns the route: `@register <payer> <registry> <fee>
/// [issuer]` registers the actor, and `@register/check` only asks.
struct Route;
impl GameModule for Route {
    fn name(&self) -> &'static str {
        "endusertestroute"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::EconomyCarveOut);
        reg.command(
            CommandSpec {
                name: "@register".into(),
                shape: ArgShape::Raw,
                switches: ["check"].into_iter().map(SmolStr::new).collect(),
                ..CommandSpec::default()
            },
            Arc::new(|ctx: &mut Ctx<'_>| -> Result<(), CmdError> {
                let raw = ctx.args.raw().to_owned();
                let mut words = raw.split_whitespace();
                let mut number = || -> Result<i64, CmdError> {
                    words
                        .next()
                        .and_then(|word| word.parse().ok())
                        .ok_or_else(|| CmdError::msg("usage"))
                };
                let (payer, registry, fee) = (number()?, number()?, number()?);
                let request = EndUserRegistration {
                    issuer: SystemPrincipalId::new(words.next().unwrap_or(ISSUER))
                        .ok_or_else(|| CmdError::msg("issuer"))?,
                    holder: ctx.actor,
                    payer: AccountId(u64::try_from(payer).map_err(|_| CmdError::msg("payer"))?),
                    registry: AccountId(
                        u64::try_from(registry).map_err(|_| CmdError::msg("registry"))?,
                    ),
                    fee: Amount::from_minor(fee),
                };
                let said = if ctx.switches.has("check") {
                    format!("{:?}", check_end_user_registration(ctx.w, &request))
                } else {
                    format!("{:?}", register_end_user(ctx, &request))
                };
                let actor = ctx.actor;
                ctx.out.info(actor, said);
                Ok(())
            }),
        )?;
        Ok(())
    }
}

fn modules() -> TestResult<Vec<Box<dyn GameModule>>> {
    Ok(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(
            EconomyModule::new()
                .with_title_issuers(vec![SystemPrincipalId::new(ISSUER).ok_or("issuer")?]),
        ),
        Box::new(Route),
    ])
}

struct Rig {
    tw: TestWorld,
    holder: ObjId,
    payer: AccountId,
    registry: AccountId,
}

fn said(tw: &mut TestWorld, actor: ObjId, line: &str) -> String {
    tw.run(actor, line)
        .iter()
        .map(|(_, event)| text_of(event))
        .collect::<Vec<_>>()
        .join("\n")
}

fn snapshot(tw: &TestWorld) -> TestResult<&EconomySnapshot> {
    Ok(tw
        .world()
        .module_state::<EconomyStore>()
        .ok_or("the economy")?
        .snapshot())
}

fn account_of(tw: &TestWorld, owner: &PrincipalId, instrument: &str) -> TestResult<AccountId> {
    Ok(snapshot(tw)?
        .accounts_of(owner)
        .find(|a| a.instrument.as_str() == instrument)
        .ok_or("the account")?
        .id)
}

fn balance(tw: &TestWorld, id: AccountId) -> TestResult<i64> {
    Ok(snapshot(tw)?
        .account(id)
        .ok_or("the account")?
        .posted_balance
        .minor())
}

fn system(name: &str) -> TestResult<PrincipalId> {
    Ok(PrincipalId::System(
        SystemPrincipalId::new(name).ok_or("a system principal")?,
    ))
}

/// A world with a funded holder, the Registry's account and no entitlement instrument.
fn rig() -> TestResult<Rig> {
    let mut tw = TestWorld::with_modules(modules()?);
    let holder = tw.spawn_player("End user");
    for line in [
        format!("@instrument/register {CREDIT}=2"),
        format!("@account/open #{}={CREDIT}", holder.0),
        format!("@account/system launch.registry={CREDIT}"),
        format!("@account/system test.faucet={CREDIT}"),
    ] {
        tw.exec_ok(GOD, &line);
    }
    let payer = account_of(&tw, &PrincipalId::Object(holder), CREDIT)?;
    let registry = account_of(&tw, &system("launch.registry")?, CREDIT)?;
    let faucet = account_of(&tw, &system("test.faucet")?, CREDIT)?;
    tw.exec_ok(GOD, &format!("@econ/mint {}={} 1000000", faucet.0, payer.0));
    Ok(Rig {
        tw,
        holder,
        payer,
        registry,
    })
}

impl Rig {
    fn register(&mut self, actor: ObjId) -> String {
        let line = format!("@register {} {} {FEE}", self.payer.0, self.registry.0);
        said(&mut self.tw, actor, &line)
    }
    fn transactions(&self) -> TestResult<usize> {
        Ok(self
            .tw
            .world()
            .module_state::<EconomyStore>()
            .ok_or("the economy")?
            .transactions()
            .count())
    }
}

#[test]
fn mw23_a_registration_is_the_fee_and_the_entitlement_in_one_transaction() -> TestResult {
    let mut r = rig()?;
    let (holder, payer, registry) = (r.holder, r.payer, r.registry);
    check_eq!(
        end_user_standing(r.tw.world(), holder),
        EndUserStanding::Unlisted
    );
    check!(entitlement(snapshot(&r.tw)?, holder).is_err());
    check!(
        snapshot(&r.tw)?.instrument(&instrument_id()?).is_none(),
        "no world has the instrument before its first registration"
    );
    // The preview writes nothing.
    let before = r.transactions()?;
    let line = format!("@register/check {} {} {FEE}", payer.0, registry.0);
    check_eq!(said(&mut r.tw, holder, &line), "Ok(())");
    check_eq!(r.transactions()?, before);
    check!(snapshot(&r.tw)?.instrument(&instrument_id()?).is_none());

    let (paid, taken) = (balance(&r.tw, payer)?, balance(&r.tw, registry)?);
    let answer = r.register(holder);
    check!(answer.starts_with("Ok("), "{answer}");
    check_eq!(r.transactions()?, before + 1, "one transaction");
    check_eq!(balance(&r.tw, payer)?, paid - FEE);
    check_eq!(balance(&r.tw, registry)?, taken + FEE);
    let def = entitlement(snapshot(&r.tw)?, holder)?.clone();
    check!(shaped(&def));
    check_eq!(def.negative_policy, NegativePolicy::SystemOnly);
    let own = account_of(&r.tw, &PrincipalId::Object(holder), CORE3_ENTITLEMENT)?;
    let source = account_of(&r.tw, &system(ISSUER)?, CORE3_ENTITLEMENT)?;
    check_eq!(balance(&r.tw, own)?, 1);
    check_eq!(balance(&r.tw, source)?, -1);
    check_eq!(
        end_user_standing(r.tw.world(), holder),
        EndUserStanding::Registered { account: own }
    );

    // Once: the second is refused, and nothing is paid or issued.
    let again = r.register(holder);
    check!(again.contains("Registered"), "{again}");
    check_eq!(r.transactions()?, before + 1);
    check_eq!(balance(&r.tw, payer)?, paid - FEE);
    check_eq!(balance(&r.tw, own)?, 1);
    check_eq!(balance(&r.tw, source)?, -1);
    Ok(())
}

#[test]
fn mw23_a_registration_is_refused_whole() -> TestResult {
    let mut r = rig()?;
    let (holder, payer, registry) = (r.holder, r.payer, r.registry);
    let before = r.transactions()?;
    let paid = balance(&r.tw, payer)?;
    // Another character may not spend the holder's account.
    let stranger = r.tw.spawn_player("Stranger");
    let answer = r.register(stranger);
    check!(answer.contains("Unavailable"), "{answer}");
    // An issuer the composition did not register issues nothing.
    let line = format!(
        "@register {} {} {FEE} some.other.issuer",
        payer.0, registry.0
    );
    let answer = said(&mut r.tw, holder, &line);
    check!(answer.contains("Unavailable"), "{answer}");
    // A fee the holder cannot cover, a fee of nothing, and the payer as its own Registry.
    for (line, expect) in [
        (
            format!("@register {} {} 100000000", payer.0, registry.0),
            "Funds",
        ),
        (
            format!("@register {} {} 0", payer.0, registry.0),
            "Unavailable",
        ),
        (
            format!("@register {} {} {FEE}", payer.0, payer.0),
            "Unavailable",
        ),
    ] {
        let answer = said(&mut r.tw, holder, &line);
        check!(answer.contains(expect), "{line}: {answer}");
    }
    // A second wallet of the holder's own is not the Registry: a fee paid to oneself is no fee.
    r.tw.exec_ok(GOD, &format!("@account/open #{}={CREDIT}", holder.0));
    let second = snapshot(&r.tw)?
        .accounts_of(&PrincipalId::Object(holder))
        .filter(|a| a.instrument.as_str() == CREDIT)
        .map(|a| a.id)
        .max()
        .ok_or("the second wallet")?;
    check_ne!(second, payer);
    let line = format!("@register {} {} {FEE}", payer.0, second.0);
    let answer = said(&mut r.tw, holder, &line);
    check!(answer.contains("Unavailable"), "{answer}");
    check_eq!(r.transactions()?, before);
    check_eq!(balance(&r.tw, payer)?, paid);
    check!(
        snapshot(&r.tw)?.instrument(&instrument_id()?).is_none(),
        "a refused registration registers nothing"
    );
    check_eq!(
        end_user_standing(r.tw.world(), holder),
        EndUserStanding::Unlisted
    );
    Ok(())
}

#[test]
fn mw23_a_frozen_registration_entitles_nobody_and_is_not_issued_again() -> TestResult {
    let mut r = rig()?;
    let holder = r.holder;
    let answer = r.register(holder);
    check!(answer.starts_with("Ok("), "{answer}");
    let own = account_of(&r.tw, &PrincipalId::Object(holder), CORE3_ENTITLEMENT)?;
    // The Registry's suspension: the account frozen, as `set_account_status` leaves it.
    r.tw.world_mut()
        .module_state_mut::<EconomyStore>()
        .ok_or("the economy")?
        .snapshot
        .account_mut(own)
        .ok_or("the entitlement account")?
        .status = AccountStatus::Frozen;
    check!(entitlement(snapshot(&r.tw)?, holder).is_err());
    check_eq!(
        end_user_standing(r.tw.world(), holder),
        EndUserStanding::Suspended { account: own }
    );
    let again = r.register(holder);
    check!(again.contains("Suspended"), "{again}");
    check_eq!(balance(&r.tw, own)?, 1);
    Ok(())
}

#[test]
fn mw23_an_empty_account_entitles_nobody_and_is_the_one_a_registration_fills() -> TestResult {
    let mut r = rig()?;
    let holder = r.holder;
    let answer = r.register(holder);
    check!(answer.starts_with("Ok("), "{answer}");
    // A second character whose entitlement account staff opened, with nothing in it.
    let other = r.tw.spawn_player("Second end user");
    for line in [
        format!("@account/open #{}={CORE3_ENTITLEMENT}", other.0),
        format!("@account/open #{}={CREDIT}", other.0),
    ] {
        r.tw.exec_ok(GOD, &line);
    }
    let empty = account_of(&r.tw, &PrincipalId::Object(other), CORE3_ENTITLEMENT)?;
    check_eq!(balance(&r.tw, empty)?, 0);
    check!(
        entitlement(snapshot(&r.tw)?, other).is_err(),
        "an account with nothing in it entitles nobody"
    );
    check_eq!(
        end_user_standing(r.tw.world(), other),
        EndUserStanding::Unlisted
    );
    // Their registration fills that account and opens no second one.
    let wallet = account_of(&r.tw, &PrincipalId::Object(other), CREDIT)?;
    let faucet = account_of(&r.tw, &system("test.faucet")?, CREDIT)?;
    r.tw.exec_ok(GOD, &format!("@econ/mint {}={} {FEE}", faucet.0, wallet.0));
    let line = format!("@register {} {} {FEE}", wallet.0, r.registry.0);
    let answer = said(&mut r.tw, other, &line);
    check!(answer.starts_with("Ok("), "{answer}");
    check_eq!(balance(&r.tw, empty)?, 1);
    check_eq!(balance(&r.tw, wallet)?, 0);
    check_eq!(
        end_user_standing(r.tw.world(), other),
        EndUserStanding::Registered { account: empty }
    );
    check_eq!(
        snapshot(&r.tw)?
            .accounts_of(&PrincipalId::Object(other))
            .filter(|a| a.instrument.as_str() == CORE3_ENTITLEMENT)
            .count(),
        1
    );
    let source = account_of(&r.tw, &system(ISSUER)?, CORE3_ENTITLEMENT)?;
    check_eq!(balance(&r.tw, source)?, -2, "two units issued, one source");
    Ok(())
}

#[test]
fn mw23_a_restart_keeps_the_registration() -> TestResult {
    let mut r = rig()?;
    let holder = r.holder;
    let answer = r.register(holder);
    check!(answer.starts_with("Ok("), "{answer}");
    let own = account_of(&r.tw, &PrincipalId::Object(holder), CORE3_ENTITLEMENT)?;
    let boot = fold_module_ops(r.tw.take_module_ops());
    let world = r.tw.into_engine().world;
    let (_, registry) = mush_testkit::boot_world();
    let mut engine = mush_core::Engine::with_modules(world, registry, modules()?)?;
    engine.modules_loaded(
        boot,
        &mut mush_core::Outbox::new(),
        &mut mush_core::engine::Effects::new(),
    )?;
    let mut tw = TestWorld::from_engine(engine);
    check!(
        !tw.world()
            .module_state::<EconomyStore>()
            .ok_or("the economy")?
            .is_frozen(),
        "the restarted economy is not frozen"
    );
    check_eq!(
        end_user_standing(tw.world(), holder),
        EndUserStanding::Registered { account: own }
    );
    check!(entitlement(snapshot(&tw)?, holder).is_ok());
    // And the restarted world still issues it once.
    let line = format!("@register {} {} {FEE}", r.payer.0, r.registry.0);
    let again = said(&mut tw, holder, &line);
    check!(again.contains("Registered"), "{again}");
    check_eq!(balance(&tw, own)?, 1);
    Ok(())
}

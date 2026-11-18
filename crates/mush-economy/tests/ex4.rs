//! **Doc 48 cut EX.4 — the player-owed obligation class** (Q-48-54; doc 48b G-4).
//!
//! Before EX.4 a daily obligation a player could not pay went `Delinquent` after one period of
//! grace (two periods), and its missed periods were simply lost: there were no arrears, and
//! reinstating forgave them. A player-owed record ([`mush_economy::create_player_obligation`])
//! keeps a grace of at least 28 periods, accrues every missed period into one one-term debt,
//! pays that debt once the account is funded, and returns from `Delinquent` on funding.
//!
//! The api takes a `Ctx`, which the test kit does not hand a test, so one test-only command
//! reaches it (e3b's `RehomeMod` pattern).

use std::sync::Arc;

use mush_core::module::{GameModule, ModError, Registrar};
use mush_core::{GOD, ObjId};
use mush_economy::{
    AccountId, Amount, EconomicClock, EconomicSchedule, EconomyStore, GracePolicy, Obligation,
    ObligationCause, ObligationId, ObligationSpec, ObligationStatus,
};
use mush_testkit::TestWorld;
use mush_testkit::fixtures::ok;
use smol_str::SmolStr;

/// The arrears debts' namespace in these tests.
const NS: &str = "ex4test";
/// The period, real seconds (the sweep fires at least this far apart).
const EVERY: i64 = 100;

/// `@ex4owe <debit>=<credit> <amount> grace=<n>`: a player-owed obligation on the real clock,
/// every [`EVERY`] seconds from [`EVERY`].
struct OweMod;

impl GameModule for OweMod {
    fn name(&self) -> &'static str {
        "ex4testowe"
    }
    fn register(&mut self, reg: &mut Registrar<'_>) -> Result<(), ModError> {
        reg.classify_commands(mush_core::CommandCompatibility::EconomyCarveOut);
        reg.command(
            mush_core::CommandSpec {
                name: "@ex4owe".into(),
                shape: mush_core::ArgShape::Raw,
                help: "@ex4owe <debit>=<credit> <amount> grace=<n>   test: a player-owed charge"
                    .into(),
                ..mush_core::CommandSpec::default()
            },
            Arc::new(
                |ctx: &mut mush_core::Ctx<'_>| -> Result<(), mush_core::CmdError> {
                    let raw = ctx.args.raw().to_string();
                    let bad = || mush_core::CmdError::msg("Usage: @ex4owe <d>=<c> <n> grace=<g>");
                    let (accounts, rest) = raw.split_once(' ').ok_or_else(bad)?;
                    let (debit, credit) = accounts.split_once('=').ok_or_else(bad)?;
                    let mut words = rest.split_whitespace();
                    let amount: i64 = words.next().and_then(|w| w.parse().ok()).ok_or_else(bad)?;
                    let grace: u8 = words
                        .next()
                        .and_then(|w| w.strip_prefix("grace="))
                        .and_then(|w| w.parse().ok())
                        .ok_or_else(bad)?;
                    let id = mush_economy::create_player_obligation(
                        ctx,
                        ObligationSpec {
                            debit_account: AccountId(debit.trim().parse().map_err(|_| bad())?),
                            credit_account: AccountId(credit.trim().parse().map_err(|_| bad())?),
                            amount: Amount::from_minor(amount),
                            schedule: EconomicSchedule::new(
                                EconomicClock::RealUtc,
                                EVERY,
                                Some(EVERY),
                                1,
                            ),
                            grace: GracePolicy::new(grace),
                            notify_on_success: false,
                            cause: ObligationCause::Ruleset {
                                namespace: SmolStr::new_static(NS),
                            },
                        },
                        NS,
                    )
                    .map_err(|e| mush_core::CmdError::msg(e.to_string()))?;
                    let actor = ctx.actor;
                    ctx.out
                        .sys_to_actor(actor, "test", format!("owe #{}", id.0));
                    Ok(())
                },
            ),
        )?;
        Ok(())
    }
}

fn boot() -> TestWorld {
    TestWorld::with_modules(vec![
        Box::new(mush_org::OrgModule::new()),
        Box::new(mush_economy::EconomyModule::with_custody(vec![])),
        Box::new(OweMod),
    ])
}

fn store(tw: &TestWorld) -> &EconomyStore {
    tw.world()
        .module_state::<EconomyStore>()
        .expect("economy store")
}

fn bal(tw: &TestWorld, id: AccountId) -> i64 {
    store(tw)
        .snapshot()
        .account(id)
        .expect("account")
        .posted_balance
        .minor()
}

fn account_of(tw: &TestWorld, owner: ObjId) -> AccountId {
    store(tw)
        .snapshot()
        .accounts()
        .find(|a| a.owner == mush_org::PrincipalId::Object(owner))
        .expect("account of owner")
        .id
}

fn obligation(tw: &TestWorld, id: ObligationId) -> Obligation {
    store(tw)
        .snapshot()
        .obligation(id)
        .expect("obligation")
        .clone()
}

/// One sweep fire at wall-clock second `t`.
fn sweep_at(tw: &mut TestWorld, t: i64) {
    tw.set_now(t);
    let _ = tw.fire_due(t.saturating_mul(1000));
}

struct Rig {
    pat: AccountId,
    quinn: AccountId,
    faucet: AccountId,
}

fn rig(tw: &mut TestWorld) -> Rig {
    let pat = tw.spawn_player("Pat");
    let quinn = tw.spawn_player("Quinn");
    ok(tw, GOD, "@instrument/register gold=0");
    ok(tw, GOD, "@account/system faucet=gold");
    ok(tw, GOD, &format!("@account/open #{}=gold", pat.0));
    ok(tw, GOD, &format!("@account/open #{}=gold", quinn.0));
    let faucet = store(tw)
        .snapshot()
        .accounts()
        .find(|a| matches!(a.owner, mush_org::PrincipalId::System(_)))
        .expect("faucet")
        .id;
    Rig {
        pat: account_of(tw, pat),
        quinn: account_of(tw, quinn),
        faucet,
    }
}

fn owe(tw: &mut TestWorld, r: &Rig, amount: i64, grace: u8) -> ObligationId {
    ok(
        tw,
        GOD,
        &format!("@ex4owe {}={} {amount} grace={grace}", r.pat.0, r.quinn.0),
    );
    store(tw)
        .snapshot()
        .obligations()
        .filter(|o| o.arrears.is_some())
        .map(|o| o.id)
        .max()
        .expect("the player-owed obligation")
}

fn debt_of(tw: &TestWorld, o: &Obligation) -> Option<Obligation> {
    o.arrears
        .as_ref()
        .and_then(|a| a.debt)
        .map(|id| obligation(tw, id))
}

/// **A week of missed days accrues as arrears, not delinquency** (the done-when's 840 cr: 168
/// IC days at 5 cr). Every miss grows one one-term debt; the record stays in grace.
#[test]
fn ex4_missed_periods_accrue_as_one_debt_and_not_delinquency() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = owe(&mut tw, &r, 500, u8::MAX);
    for k in 1..=168 {
        sweep_at(&mut tw, EVERY * k);
    }
    let o = obligation(&tw, id);
    assert!(
        matches!(o.status, ObligationStatus::Grace { missed: 168, .. }),
        "a week of misses is grace, not delinquency: {:?}",
        o.status
    );
    assert_eq!(o.missed_total, 168);
    let arrears = o.arrears.clone().expect("the class");
    assert_eq!(arrears.accrued, Amount::from_minor(168 * 500));
    let debt = debt_of(&tw, &o).expect("one arrears debt");
    assert_eq!(debt.amount, Amount::from_minor(84_000), "840 cr owed");
    assert_eq!(
        debt.debtor,
        mush_org::PrincipalId::Object(pat_of(&tw, r.pat))
    );
    assert!(debt.schedule.cadence.is_none(), "a one-term debt");
    assert!(
        !matches!(debt.status, ObligationStatus::Delinquent),
        "the debt stands payable: {:?}",
        debt.status
    );
    // One debt, not one a period.
    let debts = store(&tw)
        .snapshot()
        .obligations()
        .filter(|d| d.schedule.cadence.is_none())
        .count();
    assert_eq!(debts, 1);
    assert_eq!(bal(&tw, r.quinn), 0, "nothing moved");
}

fn pat_of(tw: &TestWorld, account: AccountId) -> ObjId {
    match &store(tw)
        .snapshot()
        .account(account)
        .expect("account")
        .owner
    {
        mush_org::PrincipalId::Object(o) => *o,
        other => panic!("not a player: {other:?}"),
    }
}

/// **Funding pays the arrears**: the next period settles, and the debt is paid whole in the
/// same fire.
#[test]
fn ex4_a_funded_account_pays_its_arrears() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = owe(&mut tw, &r, 500, u8::MAX);
    for k in 1..=3 {
        sweep_at(&mut tw, EVERY * k);
    }
    let debt = debt_of(&tw, &obligation(&tw, id)).expect("arrears");
    assert_eq!(debt.amount, Amount::from_minor(1_500));
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {}={} 10000", r.faucet, r.pat),
    );
    sweep_at(&mut tw, EVERY * 4);
    let o = obligation(&tw, id);
    assert_eq!(o.status, ObligationStatus::Active, "{:?}", o.status);
    assert_eq!(
        o.arrears.as_ref().and_then(|a| a.debt),
        None,
        "the debt was paid"
    );
    assert_eq!(obligation(&tw, debt.id).status, ObligationStatus::Satisfied);
    assert_eq!(bal(&tw, r.pat), 10_000 - 500 - 1_500);
    assert_eq!(bal(&tw, r.quinn), 2_000, "the period and the arrears");
}

/// **At least 28 periods of grace, and back on funding**: a smaller grace is raised to 28; the
/// 29th miss is delinquency, and it returns to the due pool once the account is funded, its
/// arrears paid.
#[test]
fn ex4_grace_is_at_least_28_and_funding_reinstates() {
    let mut tw = boot();
    let r = rig(&mut tw);
    let id = owe(&mut tw, &r, 500, 1);
    assert_eq!(obligation(&tw, id).grace.allowance(), 28, "raised to 28");
    for k in 1..=28 {
        sweep_at(&mut tw, EVERY * k);
    }
    assert!(matches!(
        obligation(&tw, id).status,
        ObligationStatus::Grace { missed: 28, .. }
    ));
    sweep_at(&mut tw, EVERY * 29);
    assert_eq!(obligation(&tw, id).status, ObligationStatus::Delinquent);
    let accrued = debt_of(&tw, &obligation(&tw, id)).expect("arrears").amount;
    assert_eq!(accrued, Amount::from_minor(29 * 500));
    // Delinquent attempts nothing.
    sweep_at(&mut tw, EVERY * 30);
    assert_eq!(obligation(&tw, id).missed_total, 29);
    assert_eq!(
        obligation(&tw, id).status,
        ObligationStatus::Delinquent,
        "an unfunded account is not reinstated"
    );
    // Funded: back in the due pool, and the next period settles and pays the arrears.
    ok(
        &mut tw,
        GOD,
        &format!("@econ/mint {}={} 100000", r.faucet, r.pat),
    );
    sweep_at(&mut tw, EVERY * 31);
    let o = obligation(&tw, id);
    assert_ne!(
        o.status,
        ObligationStatus::Delinquent,
        "reinstated on funding"
    );
    sweep_at(&mut tw, EVERY * 32);
    let o = obligation(&tw, id);
    assert_eq!(o.status, ObligationStatus::Active);
    assert_eq!(o.arrears.as_ref().and_then(|a| a.debt), None);
    assert_eq!(
        bal(&tw, r.quinn),
        29 * 500 + 500 * i64::from(o.settled_total)
    );
}

/// **The E4 class is unchanged**: an ordinary obligation that misses accrues nothing.
#[test]
fn ex4_an_ordinary_obligation_still_accrues_nothing() {
    let mut tw = boot();
    let r = rig(&mut tw);
    ok(
        &mut tw,
        GOD,
        &format!(
            "@obligation/create {}={} 500 every={EVERY} start={EVERY}",
            r.pat.0, r.quinn.0
        ),
    );
    for k in 1..=3 {
        sweep_at(&mut tw, EVERY * k);
    }
    let snap = store(&tw).snapshot();
    assert_eq!(snap.obligations().count(), 1, "no debt was opened");
    let o = snap.obligations().next().expect("one");
    assert_eq!(o.status, ObligationStatus::Delinquent);
    assert!(o.arrears.is_none());
}

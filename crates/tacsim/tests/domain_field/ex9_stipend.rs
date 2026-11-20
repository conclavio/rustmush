//! **Doc 48 unit EX.9 — the House stipend** (Q-48-55), over the combined M8 world booted Active.
//!
//! The launch deployment gives each of the three named Houses a stipend of 500 cr an IC day. The
//! export faucet (`tacsim.export`, a System account) pays it once an IC day into the House's
//! treasury, on the first port fire of the day:
//!
//! 1. A day turns: each House is paid exactly once, however many fires the day has.
//! 2. The faucet's account falls by what it paid (money enters through it alone).
//! 3. A day the server never fired is not paid: after a two-day jump each House is paid once.
//!
//! **Seeds** (doc 43 rule 2.3 #4): the deployment's stipend figure. Every payment is the port
//! tick's.

use mush_core::{GOD, World};
use mush_economy::{AccountId, EconomyStore};
use mush_org::{OrgId, PrincipalId, SystemPrincipalId};

use crate::excursions::{ic_now, to_ic};
use crate::m8_world::{M8World, m8_world_stipended};

const DAY: i64 = 86_400;
/// 500 cr an IC day, minor units.
const STIPEND: i64 = 50_000;

/// The account a House's stipend is paid into: its root-scope treasury, else the one the
/// deployment names.
fn treasury(world: &M8World, org: OrgId, named: AccountId) -> AccountId {
    tacsim::port::pools::treasury_of(world.rig.tw.world(), org).unwrap_or(named)
}

fn balance(w: &World, account: AccountId) -> i64 {
    w.module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().account(account).cloned())
        .map_or(0, |a| a.posted_balance.minor())
}

/// The export faucet's balance (0 before it opens).
fn faucet(w: &World) -> i64 {
    let owner = PrincipalId::System(
        SystemPrincipalId::new(tacsim::port::upkeep::EXPORT_FAUCET).expect("a system name"),
    );
    w.module_state::<EconomyStore>()
        .map(|e| {
            e.snapshot()
                .accounts_of(&owner)
                .map(|a| a.posted_balance.minor())
                .sum()
        })
        .unwrap_or(0)
}

fn houses(world: &M8World) -> Vec<AccountId> {
    let named = &world.deployment.houses;
    [&named.meridiani, &named.coprates, &named.tycho]
        .into_iter()
        .map(|house| treasury(world, house.organization, house.treasury))
        .collect()
}

fn balances(world: &M8World, accounts: &[AccountId]) -> Vec<i64> {
    accounts
        .iter()
        .map(|a| balance(world.rig.tw.world(), *a))
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex9_each_house_draws_its_stipend_once_an_ic_day() {
    let mut world = m8_world_stipended(STIPEND);
    let accounts = houses(&world);
    // Today's stipend is paid on the day's first fire; one more fire makes sure of it.
    let now = ic_now(&world.rig.tw);
    let _ = to_ic(&mut world.rig, now + 60);
    let before = balances(&world, &accounts);
    let faucet_before = faucet(world.rig.tw.world());

    // (1) The day turns: paid once, however many fires.
    let next = (ic_now(&world.rig.tw) / DAY + 1) * DAY;
    let _ = to_ic(&mut world.rig, next + 600);
    let _ = to_ic(&mut world.rig, next + 7_200);
    let _ = to_ic(&mut world.rig, next + 30_000);
    let after = balances(&world, &accounts);
    for (n, (b, a)) in before.iter().zip(&after).enumerate() {
        assert_eq!(
            a - b,
            STIPEND,
            "House {n} is paid its stipend once on the new day"
        );
    }
    // (2) The faucet paid it: at least the three stipends left it (a consumer's refund may too).
    let faucet_after = faucet(world.rig.tw.world());
    assert!(
        faucet_before - faucet_after >= 3 * STIPEND,
        "the export faucet paid the stipends: {faucet_before} -> {faucet_after}"
    );
    assert!(faucet_after < 0, "a System source falls below zero");

    // (3) Two days on in one jump: one stipend, for the day the server fires in.
    let later = (ic_now(&world.rig.tw) / DAY + 2) * DAY;
    let _ = to_ic(&mut world.rig, later + 600);
    let jumped = balances(&world, &accounts);
    for (n, (b, a)) in after.iter().zip(&jumped).enumerate() {
        assert_eq!(a - b, STIPEND, "House {n}: a day never fired is not paid");
    }
    // Staff see the faucet on the money census as a source.
    let census = crate::excursions::run(&mut world.rig.tw, GOD, "@econ/census 3");
    assert!(census.contains("tacsim.export"), "{census}");
}

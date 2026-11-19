//! **Doc 48 cut EX.6 — the escort on a passage** (48e services:G2, G3), over the composed world
//! (`driving`'s rig): a poster's Dray-P at Ceres bound for Stickney, an escort's Dray-P beside
//! her.
//!
//! - **Never set out** (G2). The poster posts and the escort takes it with a bond, and the
//!   protected hull never departs. When the term runs out the job is called off: the fee goes
//!   back to the poster and the bond to the escort, never to the poster.
//! - **No field** (G2). `field=` names a port with no field site: refused.
//! - **The departure gap** (G3). The protected hull departs while the escort is still docked at
//!   the origin: inside [`escort::TOGETHER_IC`] that is setting out, not leaving; past it, still
//!   docked at the origin, the escort has left the journey.
//!
//! **What is a fixture.** Money by `@econ/mint`, as EX.6's haul stories.

use mush_core::{GOD, ObjId};
use mush_economy::{AccountId, ContractId, ContractState, EconomyStore};
use mush_org::PrincipalId;
use tacsim::services::escort::{self, EscortStage};

use crate::driving_rig::{
    Rig, advance_to, board, file_and_launch, fire, ic_now, land_quietly, load, port_map,
    prospector, run, set_rate, stocked, sweep,
};

const CERES: &str = "ceres_port";
/// Above Q-48-13's 10,000 cr, so the escort escrows a bond of a fifth.
const FEE_CR: i64 = 20_000;

fn account(rig: &Rig, who: ObjId) -> AccountId {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .accounts_of(&PrincipalId::Object(who))
                .find(|a| a.instrument.as_str() == tacsim::CREDIT)
                .map(|a| a.id)
        })
        .expect("a credit account")
}

/// Posted balance less what is held.
fn balance(rig: &Rig, account: AccountId) -> i64 {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| {
            e.snapshot()
                .account(account)
                .map(|a| a.posted_balance.minor() - a.held_amount.minor())
        })
        .expect("the account")
}

fn mint(rig: &mut Rig, to: AccountId, minor: i64) {
    let treasury = port_map(&rig.tw, CERES).treasury.expect("a treasury");
    let out = run(
        &mut rig.tw,
        GOD,
        &format!("@econ/mint {treasury}={} {minor}", to.0),
    );
    assert!(!out.contains("Usage"), "{out}");
}

fn state(rig: &Rig, contract: u64) -> ContractState {
    rig.tw
        .world()
        .module_state::<EconomyStore>()
        .and_then(|e| e.snapshot().contract(ContractId(contract)).map(|c| c.state))
        .expect("the contract")
}

fn job(rig: &Rig, contract: u64) -> escort::EscortJob {
    escort::escort_of(rig.tw.world(), contract).expect("the escort")
}

/// The poster and the escort, each with a funded wallet and a Dray-P docked at Ceres; the
/// poster's escort to Stickney, posted and taken. `(poster, protected, escort, with, contract)`.
fn taken(rig: &mut Rig) -> (ObjId, ObjId, ObjId, ObjId, u64) {
    let (poster, row) = prospector(rig, "Poster", 91, "Ward");
    let (guard, with_row) = prospector(rig, "Guard", 92, "Sentry");
    let (hull, with) = (ObjId(row.hull), ObjId(with_row.hull));
    let (poster_wallet, guard_wallet) = (account(rig, poster), account(rig, guard));
    mint(rig, poster_wallet, FEE_CR * 100 * 5);
    mint(rig, guard_wallet, FEE_CR * 100 * 5);
    let posted = run(
        &mut rig.tw,
        poster,
        &format!("+escort/post #{} to=stickney fee={FEE_CR} go", hull.0),
    );
    assert!(posted.contains("posted"), "{posted}");
    let contract = escort::escorts(rig.tw.world())
        .into_iter()
        .filter(|j| j.poster == poster)
        .map(|j| j.contract)
        .max()
        .expect("the escort job");
    let accepted = run(
        &mut rig.tw,
        guard,
        &format!("+escort/accept {contract} with=#{} go", with.0),
    );
    assert!(accepted.contains("You take escort"), "{accepted}");
    let queued = crate::driving_rig::drain(&mut rig.tw);
    land_quietly(rig, &queued);
    assert_eq!(job(rig, contract).stage, EscortStage::Taken);
    (poster, hull, guard, with, contract)
}

/// Several port fires, what they stage landed, and the economy's sweep settled.
fn settle(rig: &mut Rig, n: usize) {
    for _ in 0..n {
        let (_, queued) = fire(rig);
        land_quietly(rig, &queued);
        sweep(&mut rig.tw, 60);
        crate::in10_pools::settle(rig, 1);
    }
}

/// **48e services:G2**: a poster who never sets out does not keep the escort's bond. When the
/// term runs out on a journey that never began, the job is called off: the poster's fee and the
/// escort's bond go back where they came from. And `field=` at a port with no field is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex6_an_escort_whose_journey_never_begins_is_called_off_with_the_bond_back() {
    let mut rig = stocked();
    let (poster, hull, guard, _, contract) = taken(&mut rig);
    let (poster_wallet, guard_wallet) = (account(&rig, poster), account(&rig, guard));
    // The poster's fee and the escort's bond are escrowed.
    let bond = job(&rig, contract).bond_minor;
    assert_eq!(bond, FEE_CR * 100 / 5, "a fifth of a fee above 10,000 cr");
    let (poster_before, guard_before) = (
        balance(&rig, poster_wallet) + FEE_CR * 100,
        balance(&rig, guard_wallet) + bond,
    );

    // `field=` at Ceres, which has no field, is refused: the journey is read before the open
    // job on her is.
    let own_field = run(
        &mut rig.tw,
        poster,
        &format!("+escort/post #{} field=ceres fee=100 go", hull.0),
    );
    assert!(
        own_field.contains("There is no field at ceres"),
        "a port with no field site names no sortie: {own_field}"
    );

    // The term runs out with her never undocked.
    let due = job(&rig, contract).due_ic.expect("a term");
    let (_, queued) = advance_to(&mut rig, due + 60);
    land_quietly(&mut rig, &queued);
    settle(&mut rig, 4);
    let ended = job(&rig, contract);
    assert_eq!(
        ended.stage,
        EscortStage::Closed,
        "called off, not failed: {ended:?}"
    );
    assert!(
        matches!(state(&rig, contract), ContractState::Cancelled),
        "the contract unwound: {:?}",
        state(&rig, contract)
    );
    assert_eq!(
        balance(&rig, guard_wallet),
        guard_before,
        "the escort's bond is back"
    );
    assert_eq!(
        balance(&rig, poster_wallet),
        poster_before,
        "the poster's fee is back, and nothing of the bond"
    );
}

/// **48e services:G3**: the protected hull departs while the escort is still docked at the
/// origin. Inside the departure gap the job keeps watching; past it, with the escort still at
/// the origin, she has left the journey.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn ex6_an_escort_still_at_the_origin_inside_the_departure_gap_has_not_left() {
    let mut rig = stocked();
    let (poster, hull, _, _, contract) = taken(&mut rig);
    // At 24:1 the gap is 150 real seconds, so a port fire lands inside it.
    set_rate(&mut rig.tw, 24);
    let row = port_map(&rig.tw, CERES)
        .hulls
        .into_iter()
        .find(|row| row.hull == hull.0)
        .expect("her row");
    load(&mut rig, &row, 60, 40, 0xe6c0);
    board(&mut rig, poster);
    let _ = file_and_launch(&mut rig, poster, hull, "+ship/undock stickney");
    // Her journey begins at the next watch.
    let mut started = None;
    for _ in 0..10 {
        let watching = job(&rig, contract);
        if let Some(watch) = watching.watch.as_ref() {
            started = Some(watch.started_ic);
            break;
        }
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    let started = started.expect("the watch starts when her journey does");
    // A fire inside the gap: the escort is still docked at Ceres, setting out.
    let (_, queued) = fire(&mut rig);
    land_quietly(&mut rig, &queued);
    assert!(ic_now(&rig.tw) - started <= escort::TOGETHER_IC);
    let inside = job(&rig, contract);
    assert_eq!(
        inside.stage,
        EscortStage::Watching,
        "docked at the origin inside the gap is not leaving: {inside:?}"
    );
    // Past the gap, still docked at the origin: she has left the journey.
    let (_, queued) = advance_to(&mut rig, started + escort::TOGETHER_IC + 60);
    land_quietly(&mut rig, &queued);
    for _ in 0..3 {
        let (_, queued) = fire(&mut rig);
        land_quietly(&mut rig, &queued);
    }
    let past = job(&rig, contract);
    assert_eq!(past.stage, EscortStage::Failed, "{past:?}");
    assert_eq!(past.why.as_deref(), Some(escort::WHY_LEFT));
}

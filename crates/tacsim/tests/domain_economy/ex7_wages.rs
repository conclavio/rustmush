//! **Doc 48 cut EX.7 — crew wages and crew for hire** (Q-48-14, -33, -54), over the composed
//! lane (`driving`'s rig) at the live rate, 24:1, where an IC day is one real hour.
//!
//! 1. **A wage paid:** a captain hires Ceres' rating and officer through `+sortie/crew`; each
//!    hire owes 10 cr or 20 cr an IC day to the hiring port's authority, paid from her account
//!    day by day, and `+sortie/contract` shows it.
//! 2. **A wage missed under grace:** a broke captain's hire misses its days. They accrue as
//!    arrears and the wage stays in grace, the contract standing; once she is funded the
//!    arrears are paid.
//! 3. **A player lists themselves for crew hire:** `+sortie/available` puts them on the port's
//!    job board, as a technician among their duties, until they withdraw.
//!
//! **What is a fixture.** The captains' money (minted by staff); the hires, the wages and the
//! board are the production path, and the clock runs at the live rate.

use mush_core::ObjId;
use mush_economy::{AccountId, Obligation, ObligationStatus};
use mush_org::PrincipalId;

use crate::driving_rig::{
    Rig, command, land_composed, pilot, port_map, run, set_rate, stocked, sweep, teleport,
};
use crate::hull_resale_ex2::{account_of, balance, captain, mint};

/// **A captain at Ceres with her own credit account**: a Co-op charter (her enrolment opens the
/// account), so she also pays her charter's berth, which [`berth_paid`] reads.
pub(crate) fn employer(rig: &mut Rig, name: &str, seed: i64, hull: &str) -> ObjId {
    captain(rig, name, seed, hull).0
}

/// What `who` has paid in berth so far, minor units.
pub(crate) fn berth_paid(rig: &Rig, who: ObjId) -> i64 {
    crate::driving_rig::berth_paid(rig.tw.world(), &PrincipalId::Object(who))
}

/// Staff take `who`'s whole balance away: a captain who cannot pay.
pub(crate) fn broke(rig: &mut Rig, who: ObjId) {
    let account = account_of(rig, who);
    let left = balance(rig, account);
    let sink = port_map(&rig.tw, "ceres_port")
        .treasury
        .expect("a treasury");
    if left > 0 {
        let out = run(
            &mut rig.tw,
            mush_core::GOD,
            &format!("@econ/pay {}={sink} {left}", account.0),
        );
        assert!(!out.contains("Usage"), "{out}");
    }
    assert_eq!(balance(rig, account), 0);
}

/// An IC day at 24:1, real seconds.
pub(crate) const IC_DAY_REAL: i64 = 3_600;
/// The shipped rating, a unit operator.
pub(crate) const MENDES: &str = "Ilya Mendes";
/// The shipped officer, a detachment commander.
pub(crate) const OKONKWO: &str = "Sana Okonkwo";

/// The NPC named `name`.
pub(crate) fn npc(rig: &Rig, name: &str) -> ObjId {
    rig.tw
        .world()
        .iter()
        .find(|o| o.kind == mush_core::ObjKind::Npc && o.name.as_str() == name)
        .map(|o| o.id)
        .unwrap_or_else(|| panic!("{name} stands in the world"))
}

/// The port authority's account at Ceres, which keeps the hiring hall.
pub(crate) fn authority(rig: &Rig) -> AccountId {
    let map = port_map(&rig.tw, "ceres_port");
    let row = map
        .orgs
        .iter()
        .find(|(id, _)| id.as_str() == "freeport_registry")
        .map(|(_, row)| row.clone())
        .expect("Ceres' authority");
    AccountId(row.account.expect("its account"))
}

/// **Hire `who`'s `npc`** where the NPC stands, through the line `+sortie/crew` prints; what the
/// offers said.
pub(crate) fn hire(rig: &mut Rig, who: ObjId, npc: ObjId) -> String {
    let room = rig
        .tw
        .world()
        .get(npc)
        .and_then(|o| o.location)
        .expect("the crew member stands in a room");
    teleport(&mut rig.tw, who, room);
    let offers = run(&mut rig.tw, who, "+sortie/crew");
    let accept = offers
        .lines()
        .find_map(|line| {
            line.trim()
                .strip_prefix("Accept: ")
                .filter(|line| line.contains(&format!("#{}=", npc.0)))
        })
        .unwrap_or_else(|| panic!("#{} is on offer: {offers}", npc.0))
        .to_owned();
    let (said, queued) = command(rig, who, &accept);
    assert_eq!(queued.len(), 1, "one hire: {said}");
    land_composed(rig, &queued[0]).expect("the hire lands");
    offers
}

/// `npc`'s wage, as `who` owes it.
pub(crate) fn wage(rig: &Rig, who: ObjId, npc: ObjId) -> Option<Obligation> {
    let ns = tacsim::detachment::crew::wages::wage_namespace(npc);
    mush_economy::player_obligations_of(rig.tw.world(), &PrincipalId::Object(who), &ns)
        .into_iter()
        .find(|o| o.schedule.cadence.is_some())
}

/// One IC day at 24:1: every tick fires once.
pub(crate) fn day(rig: &mut Rig) {
    sweep(&mut rig.tw, IC_DAY_REAL);
}

/// **A wage paid**: the rating's 10 cr and the officer's 20 cr an IC day go from the captain to
/// Ceres' authority, day by day, and her contract shows it.
#[test]
fn ex7_a_hire_is_paid_its_wage_by_the_ic_day() {
    let mut rig = stocked();
    let who = employer(&mut rig, "Employer", 1, "Salt Moth");
    mint(&mut rig, who, 100_000);
    set_rate(&mut rig.tw, 24);
    let (mendes, okonkwo) = (npc(&rig, MENDES), npc(&rig, OKONKWO));
    let offers = hire(&mut rig, who, mendes);
    assert!(offers.contains("; wage 10.00cr an IC day"), "{offers}");
    assert!(
        offers.contains(
            "A hire's wage is 10.00cr an IC day for a rating and 20.00cr for an officer, paid to the hiring port."
        ),
        "{offers}"
    );
    let _ = hire(&mut rig, who, okonkwo);
    sweep(&mut rig.tw, 60);

    let rating = wage(&rig, who, mendes).expect("the rating's wage is charged");
    let officer = wage(&rig, who, okonkwo).expect("the officer's wage is charged");
    assert_eq!(rating.amount.minor(), 1_000, "10 cr an IC day");
    assert_eq!(officer.amount.minor(), 2_000, "20 cr an IC day");
    let hall = authority(&rig);
    for o in [&rating, &officer] {
        assert_eq!(
            o.credit_account, hall,
            "paid to the hiring port's authority"
        );
        assert_eq!(
            o.debit_account,
            account_of(&rig, who),
            "from her own account"
        );
        assert!(
            o.grace.allowance() >= mush_economy::PLAYER_OWED_MIN_GRACE,
            "EX.4's grace: at least 28 periods"
        );
    }

    let (mine, theirs) = (balance(&rig, account_of(&rig, who)), balance(&rig, hall));
    let berthed = berth_paid(&rig, who);
    for _ in 0..3 {
        day(&mut rig);
    }
    let rating = wage(&rig, who, mendes).expect("the rating's wage");
    let officer = wage(&rig, who, okonkwo).expect("the officer's wage");
    assert_eq!(rating.settled_total, 3, "three IC days paid: {rating:?}");
    assert_eq!(officer.settled_total, 3, "three IC days paid: {officer:?}");
    assert_eq!(rating.missed_total + officer.missed_total, 0);
    // Her charter's berth at Ceres is paid to the same authority over the same days.
    let berth = berth_paid(&rig, who) - berthed;
    assert_eq!(
        balance(&rig, account_of(&rig, who)),
        mine - 3 * 3_000 - berth,
        "30 cr a day for the two, beside her berth"
    );
    assert_eq!(
        balance(&rig, hall),
        theirs + 3 * 3_000 + berth,
        "the hall is paid"
    );

    let contract = run(
        &mut rig.tw,
        who,
        &format!("+sortie/contract #{}", okonkwo.0),
    );
    assert!(
        contract.contains("Wage: 20.00cr an IC day to ")
            && contract.contains("; 0.00cr in arrears."),
        "{contract}"
    );
}

/// **A wage missed under grace**: a broke captain's rating accrues each missed IC day as arrears,
/// the wage stays in grace and the contract stands; funded, she pays the arrears.
#[test]
fn ex7_a_missed_wage_accrues_under_grace_and_is_paid_once_funded() {
    let mut rig = stocked();
    let who = employer(&mut rig, "Broke", 2, "Red Moth");
    broke(&mut rig, who);
    set_rate(&mut rig.tw, 24);
    let mendes = npc(&rig, MENDES);
    let _ = hire(&mut rig, who, mendes);
    sweep(&mut rig.tw, 60);
    assert_eq!(balance(&rig, account_of(&rig, who)), 0, "she cannot pay");
    assert!(wage(&rig, who, mendes).is_some(), "her wage is charged");

    for _ in 0..40 {
        if wage(&rig, who, mendes).is_some_and(|o| o.missed_total >= 5) {
            break;
        }
        day(&mut rig);
    }
    let o = wage(&rig, who, mendes).expect("the wage");
    assert_eq!(o.missed_total, 5, "five IC days missed");
    assert!(
        matches!(o.status, ObligationStatus::Grace { .. }),
        "in grace, not delinquent: {:?}",
        o.status
    );
    let arrears = tacsim::detachment::crew::wages::arrears_of(rig.tw.world(), who, mendes);
    assert_eq!(arrears, 5_000, "50 cr in arrears");
    // The contract stands: a missed wage never unhires.
    let contract = run(&mut rig.tw, who, &format!("+sortie/contract #{}", mendes.0));
    assert!(contract.contains("Crew contract"), "{contract}");
    assert!(contract.contains("; 50.00cr in arrears."), "{contract}");

    // Funded, she pays the arrears in the next fire, and the day's wage.
    mint(&mut rig, who, 100_000);
    day(&mut rig);
    assert_eq!(
        tacsim::detachment::crew::wages::arrears_of(rig.tw.world(), who, mendes),
        0,
        "the arrears are paid"
    );
    let o = wage(&rig, who, mendes).expect("the wage");
    assert_eq!(o.missed_total, 5, "nothing more missed");
    assert!(o.settled_total >= 1, "the day's wage is paid: {o:?}");
}

/// **A player lists themselves for crew hire**: on the port's board while they stand there, as a
/// technician among their duties; gone when they withdraw.
#[test]
fn ex7_a_player_lists_themselves_for_crew_hire_on_the_board() {
    let mut rig = stocked();
    let who = pilot(&mut rig, "Hand", 3);
    let listed = run(&mut rig.tw, who, "+sortie/available technician,mining");
    assert!(
        listed.contains("Listed for crew hire: rating (mining, technician) at ")
            && listed.contains(", asking 10.00cr an IC day."),
        "{listed}"
    );
    let board = run(&mut rig.tw, who, "+market/jobs crew market=ceres");
    assert!(
        board.contains("Crew: rating for hire (mining, technician). Pays 10.00cr to open."),
        "{board}"
    );
    assert!(
        board.contains("paid by the IC day; posted by Hand. Take it: page Hand"),
        "{board}"
    );
    let shown = run(&mut rig.tw, who, "+sortie/available");
    assert!(shown.contains("shown there on its board."), "{shown}");
    assert!(
        run(&mut rig.tw, who, "+sortie/available pilot").contains("Usage: +sortie/available"),
        "an unknown duty is refused"
    );

    // An officer's listing asks the officer's wage.
    let officer = run(&mut rig.tw, who, "+sortie/available combat officer");
    assert!(
        officer.contains("officer (combat)") && officer.contains("20.00cr"),
        "{officer}"
    );
    let board = run(&mut rig.tw, who, "+market/jobs crew market=ceres");
    assert!(
        board.contains("Crew: officer for hire (combat). Pays 20.00cr to open."),
        "{board}"
    );

    // Withdrawn, the board is empty of them.
    let gone = run(&mut rig.tw, who, "+sortie/unavailable");
    assert!(
        gone.contains("You are no longer listed for crew hire."),
        "{gone}"
    );
    let board = run(&mut rig.tw, who, "+market/jobs crew market=ceres");
    assert!(!board.contains("posted by Hand"), "{board}");
    assert!(
        run(&mut rig.tw, who, "+sortie/unavailable").contains("You are not listed for crew hire."),
        "nothing left to withdraw"
    );
}

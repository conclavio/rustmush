//! **Doc 41 story (c), the charter half, with the bot over SSH** (§4.9; cuts RS.5 and RS.9c), at
//! 256 (Ahuna Flank) and 512 (Urvara Reach): two players, each typing over their own session.
//! `field_rs5`'s charter story, typed.
//!
//! 1. **A** charters a Dray-P, leases a prospector, flies to the site and sends it down the ramp;
//!    it is damaged there (its power out, its chassis worn), and A sails home to Ceres without it
//!    (`+ship/lift ceres leave=1`), broke.
//! 2. No departure for the dormancy window: the Co-op's recall takes A's Dray-P back and
//!    repossesses the prospector **where it lies**, posting its retrieval at Tycho Mutual's quote;
//!    the deposit pays the restoration (less the wear allowance) and the fee as far as it goes, and
//!    the rest is A's one-term debt (A is told the allowance and the debt).
//! 3. **B** enrols, flies to the site, reads the job on `+salvage/jobs`, accepts it, tows and loads
//!    the prospector (retrieval custody: `+bay` reads "Your right: retrieval"), sails home, and the
//!    shed's hand-back at the docking posts the delivery: B is paid.
//! 4. A at Occator's counter: the debt is due, not delinquent, for 28 IC days; past them `+debts`
//!    reads it delinquent and `+tv/lease go` is refused naming the amount (the desk reinstates
//!    it); `+debts/pay` refuses short, and pays once A is funded.
//!
//! **What is a fixture** (doc 36a §8's seams): God's: the prospector's damage (`@unit/damage`) and
//! its row on the site's index (`@tacsim/site leave`, which the field that did it would have
//! written), A's spend-down and later funds (`@econ/pay`, `@econ/mint`), A's walk to Occator's
//! counter (`@teleport`), and the world's rate. **The dormancy window** is lowered to
//! [`WINDOW_S`] real seconds on the booted engine through tacsim's test-only seam
//! (`tacsim::store::lower_dormancy_floor_for_test`, the `test-dormancy-floor` feature that only
//! this crate's dev-dependency turns on): the programme's floor is one real day, which no bot run
//! waits out. No config key reaches it, and the production floor is unchanged. A waits the window
//! out at the boot's rate (24), so the prospector's 28-IC-day clocks never run inside it; B
//! enrols only after the repossession, and flies out and back inside the window, so B's own
//! charter is never the one recalled.

use std::time::{Duration, Instant};

use mush_bot::mine::Milestone;
use mush_core::ObjId;

use crate::fr7_round::cid;
use crate::grounds_rig::Ground;
use crate::rs9_story_b::{credit_of, enrol, fly, id_after, named, tow_in, typed, until};
use crate::{arena, mine};

/// **The dormancy window, real seconds**, on the test root: long enough that B's flight out and
/// back (one departure to the next) never spans it, short enough to wait out once.
const WINDOW_S: u32 = 900;

/// The two players: A the lessee, B the retriever.
#[derive(Clone, Copy)]
pub(crate) struct Pair {
    pub(crate) a: mine::Miner,
    pub(crate) b: mine::Miner,
}

/// What the fixture read off the booted world: A, Occator's counter at Ceres, and the ramp's cell.
type Made = (ObjId, ObjId, (i64, i64));

/// Occator's counter, Ceres's C07 exchange.
fn ceres_counter(engine: &mush_core::Engine) -> ObjId {
    let lane = tacsim::pack::load_pack(tacsim::definitions::content::LANE_JSON).expect("the lane");
    let Some(tacsim::pack::PackEntry::Port(def)) = lane.entry(&cid("ceres_port")) else {
        panic!("Ceres is a port entry");
    };
    let key = tacsim::port::room_key(&def.area, &format!("{}.room.c07_exchange", def.area))
        .expect("a room key");
    tacsim::port::apply::room_by_key(&engine.world, &key).expect("Ceres's C07 exchange")
}

/// **The prospector's damage**, as a mutation plan's JSON (`field_rs5`'s): its power destroyed and
/// its chassis damaged, 40 internals left — a casualty, not a wreck.
fn damage_plan() -> String {
    let plan = mush_tactics::UnitMutationPlan::new(
        vec![
            mush_tactics::UnitMutationOp::SetComponentCondition {
                component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
                condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
            },
            mush_tactics::UnitMutationOp::SetSectionCondition {
                section: mush_tactics::SectionId::new("chassis").expect("a section"),
                condition: mush_tactics::ConditionId::new("damaged").expect("a condition"),
            },
            mush_tactics::UnitMutationOp::SetCell {
                addr: mush_tactics::CellAddr::Section {
                    section: mush_tactics::SectionId::new("chassis").expect("a section"),
                    cell: mush_tactics::CellId::new(tacsim::definitions::unit::INTERNALS_CELL)
                        .expect("a cell"),
                },
                value: 40,
            },
        ],
        None,
        Some("bot-story:rs9c-charter".into()),
    )
    .expect("a plan");
    serde_json::to_string(&plan).expect("the plan's JSON")
}

/// God moves all but a little of `account`'s available credit to a fresh sink, in shrinking
/// chunks until each is refused (held deposits are not available; story (b)'s spend-down).
async fn spend_down(god: &mut arena::Client, account: u64) {
    let sink = god
        .say(
            "@account/system rs9c_sink=tacsim.credit",
            "Opened account #",
        )
        .await;
    let sink = id_after(&sink, "Opened account #");
    for chunk in [500_000_u64, 100_000, 10_000] {
        loop {
            let (index, paid) = god
                .answer(
                    &format!("@econ/pay {account}={sink} {chunk}"),
                    &["Paid: transaction #", "insufficient", "Usage", "denied"],
                )
                .await;
            match index {
                Some(0) => {}
                Some(1) => break,
                _ => panic!("God spends the account down:\n{paid}"),
            }
        }
    }
}

/// **Story (c)'s charter half on `ground`**, the pair `pair`.
#[allow(clippy::too_many_lines)]
pub(crate) async fn story_c_charter(pair: Pair, ground: &'static Ground) {
    let started = Instant::now();
    let site = ground.entry;
    let stamp = |what: &str| {
        println!(
            "rs9c-charter {site}: +{} s {what}",
            started.elapsed().as_secs()
        )
    };
    let mut made = None;
    let (rig, _) = mine::boot_with(
        &[pair.a, pair.b],
        |cfg| {
            cfg.tacsim.recovery.inspect_s = 30;
            cfg.tacsim.recovery.attach_s = 30;
            cfg.tacsim.recovery.load_s = 60;
        },
        |engine| {
            // The dormancy window below the programme's floor, through the test-only seam on the
            // booted engine; no shipped config reaches it.
            assert!(
                tacsim::store::lower_dormancy_floor_for_test(&mut engine.world, WINDOW_S),
                "the layer is booted"
            );
            let (x, y) = tacsim::surface::excursion::site_pad_cm(&engine.world, &cid(ground.entry));
            let side = i64::from(tacsim::surface::ground::SITE_CELL_CM);
            let ramp = (x.div_euclid(side), y.div_euclid(side) - 4);
            made = Some::<Made>((named(engine, pair.a.character), ceres_counter(engine), ramp));
            Vec::new()
        },
    )
    .await;
    let (a_id, counter, ramp) = made.expect("the fixture ran");
    let mut god = arena::Client::login(&rig, arena::GOD_ACCOUNT, arena::GOD_PASSWORD, "God").await;
    let mut a = arena::Client::login(
        &rig,
        pair.a.account,
        arena::PILOT_PASSWORD,
        pair.a.character,
    )
    .await;
    let mut b = arena::Client::login(
        &rig,
        pair.b.account,
        arena::PILOT_PASSWORD,
        pair.b.character,
    )
    .await;

    // 1. A enrols at the boot's rate, flies out and sends the prospector down the ramp.
    let mut a_lane = enrol(&mut a, pair.a, started).await;
    mine::lift_rate(&mut god, mine::LIFTED_RATE).await;
    fly(&mut a, &mut a_lane, started, |lane| {
        lane.reached(Milestone::Launched)
    })
    .await;
    let (_, rig1) = typed(&mut a, "+tv/1", &["Rig 1: "]).await;
    let mark = rig1
        .split("Rig 1: ")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .expect("rig 1's mark")
        .trim()
        .to_owned();
    // The mark's own line (`OWP-TV-01(#548)`): other lines (an NPC's) may carry a `(#` first.
    let hit = format!("{mark}(#");
    let (_, found) = god
        .answer(&format!("@find {mark}"), &[hit.as_str(), "Nothing"])
        .await;
    let prospect = ObjId(u32::try_from(id_after(&found, &hit)).expect("an id"));
    let p = format!("#{}", prospect.0);
    stamp(&format!("A's prospector {mark} ({p}) is down the ramp"));
    // The damage, and its row on the site's index (the field that did it would have written it).
    god.say(&format!("@unit/damage {p}={}", damage_plan()), "Applied ")
        .await;
    god.say(
        &format!(
            "@tacsim/site {site} leave {p} casualty {},{}",
            ramp.0, ramp.1
        ),
        "Site record write requested",
    )
    .await;
    let indexed = Instant::now() + Duration::from_secs(60);
    loop {
        god.idle(Duration::from_secs(2)).await;
        let (_, record) = god
            .answer(&format!("@tacsim/site {site}"), &["Objects:"])
            .await;
        if record.contains(&format!("Unit {p}:")) {
            break;
        }
        assert!(Instant::now() < indexed, "{p} never indexed:\n{record}");
    }
    stamp("the prospector is damaged and on the index");

    // A sails home without it, then is spent down so the charge outruns the deposit.
    let from = a.seen.len();
    let (_, said) = typed(
        &mut a,
        "+ship/lift ceres leave=1",
        &["Clearance", "lifts", "cannot", "refuse"],
    )
    .await;
    println!(
        "rs9c-charter: A> +ship/lift ceres leave=1\n{}",
        said.trim_end()
    );
    assert!(
        a.watch(
            from,
            &mush_bot::mine::lines::docks(&a_lane.settings().hull),
            Duration::from_secs(20 * 60)
        )
        .await,
        "A never docked at Ceres:\n{}",
        mush_bot::tail(&a.seen, 4 * 1024)
    );
    stamp("A docked at Ceres");
    let (a_account, _) = credit_of(&mut a).await;
    spend_down(&mut god, a_account).await;

    // 2. The window without a departure, at the boot's rate: the recall repossesses the
    //    prospector where it lies, and posts its retrieval.
    mine::lift_rate(&mut god, 24).await;
    let from = a.seen.len();
    // `REPOSSESSED_LEFT`'s middle.
    let left = " where it lies on ";
    assert!(tacsim::port::tv::REPOSSESSED_LEFT.contains(left));
    assert!(
        a.watch(
            from,
            left,
            Duration::from_secs(u64::from(WINDOW_S) + 10 * 60)
        )
        .await,
        "the prospector was never repossessed:\n{}",
        mush_bot::tail(&a.seen, 4 * 1024)
    );
    let heard = a.seen[from..].to_owned();
    assert!(
        heard.contains("wear allowance") && heard.contains("as a debt (+debts)"),
        "A is told the allowance and the debt:\n{heard}"
    );
    let (_, owed) = typed(&mut a, "+debts", &["Your debts:", "You owe nothing."]).await;
    let debt = id_after(&owed, "  #");
    assert!(owed.contains("(due)"), "owed, not yet delinquent:\n{owed}");
    stamp(&format!("repossessed where it lies; A owes debt #{debt}"));

    // 3. B enrols, flies out, accepts the job, tows and loads the prospector, and brings it home.
    let mut b_lane = enrol(&mut b, pair.b, started).await;
    mine::lift_rate(&mut god, mine::LIFTED_RATE).await;
    fly(&mut b, &mut b_lane, started, |lane| {
        lane.reached(Milestone::Launched)
    })
    .await;
    stamp("B's rig is down the ramp");
    let board = until(
        &mut b,
        "+salvage/jobs",
        &[p.as_str(), "No retrieval jobs"],
        5,
        Duration::from_secs(60),
    )
    .await;
    let job = board
        .lines()
        .find(|line| line.contains(&p))
        .and_then(|line| {
            line.split('#').nth(1).map(|rest| {
                rest.chars()
                    .take_while(char::is_ascii_digit)
                    .collect::<String>()
            })
        })
        .unwrap_or_else(|| panic!("the job on the board:\n{board}"));
    let (index, said) = typed(
        &mut b,
        &format!("+salvage/accept #{job} go"),
        &["You accept job"],
    )
    .await;
    assert_eq!(index, Some(0), "B accepts:\n{said}");
    tow_in(&mut b, &mut god, site, prospect).await;
    until(
        &mut b,
        &format!("+bay {p}"),
        &["Your right: retrieval"],
        5,
        Duration::from_secs(10 * 60),
    )
    .await;
    stamp("the prospector is in B's bay under the retrieval");
    let lifted = mine::lift_rate(&mut god, 3_600).await;
    stamp(&format!("rate: {}", lifted.lines().next().unwrap_or("")));
    let from = b.seen.len();
    let (_, said) = typed(
        &mut b,
        "+ship/lift ceres leave=1",
        &["Clearance", "lifts", "cannot", "refuse"],
    )
    .await;
    println!(
        "rs9c-charter: B> +ship/lift ceres leave=1\n{}",
        said.trim_end()
    );
    // The shed's hand-back at the docking posts the delivery, and the fee follows.
    let paid = tacsim::surface::retrieval::TOLD_PAID
        .split("{fee}")
        .next()
        .expect("the paid line's head")
        .to_owned();
    assert!(
        b.watch(from, &paid, Duration::from_secs(20 * 60)).await,
        "B was never paid:\n{}",
        mush_bot::tail(&b.seen, 4 * 1024)
    );
    stamp("B was paid at Ceres");

    // 4. A at Occator's counter: past the 28 IC days' grace the debt is delinquent, the desk
    //    refuses a lease naming it (and reinstates it), and A pays it once funded.
    let (_, said) = typed(&mut a, "+ship/leave", &["leave", "Leave", "aboard", "lock"]).await;
    println!(
        "rs9c-charter: A> +ship/leave
{}",
        said.trim_end()
    );
    let (_, moved) = god
        .answer(
            &format!("@teleport #{}=#{}", a_id.0, counter.0),
            &["Teleported", "teleport", "Done", "denied", "Huh", "can't"],
        )
        .await;
    println!(
        "rs9c-charter: God> @teleport A to C07: {}",
        moved.trim_end()
    );
    let delinquent = until(
        &mut a,
        "+debts",
        &["(delinquent)", "Your debts:", "You owe nothing."],
        10,
        Duration::from_secs(20 * 60),
    )
    .await;
    println!("rs9c-charter: past the grace:\n{}", delinquent.trim_end());
    let (index, refused) = typed(
        &mut a,
        "+tv/lease go",
        &["will not deal with you", "lease", "Lease"],
    )
    .await;
    assert_eq!(
        index,
        Some(0),
        "the desk refuses the delinquent lessee:\n{refused}"
    );
    assert!(refused.contains("delinquent debts"), "{refused}");
    let (_, listed) = typed(&mut a, "+debts", &["Your debts:", "You owe nothing."]).await;
    assert!(
        listed.contains(&format!("#{debt}")) && listed.contains("(due)"),
        "the desk reinstated it:\n{listed}"
    );
    let pay = format!("+debts/pay {debt}");
    let (index, short) = typed(&mut a, &pay, &["cannot cover", "paid"]).await;
    assert_eq!(index, Some(0), "A cannot cover it yet:\n{short}");
    let float = god
        .say(
            "@account/system rs9c_float=tacsim.credit",
            "Opened account #",
        )
        .await;
    let float = id_after(&float, "Opened account #");
    god.say(
        &format!("@econ/mint {float}={a_account} 10000000"),
        "Minted",
    )
    .await;
    let (index, paid) = typed(
        &mut a,
        &pay,
        &[&format!("Debt #{debt} paid"), "cannot cover"],
    )
    .await;
    assert_eq!(index, Some(0), "A pays the debt:\n{paid}");
    let (index, none) = typed(&mut a, "+debts", &["You owe nothing.", "Your debts:"]).await;
    assert_eq!(index, Some(0), "nothing owed:\n{none}");
    stamp("A's debt is paid");
    for client in [a, b, god] {
        client.close().await;
    }
    drop(rig);
}

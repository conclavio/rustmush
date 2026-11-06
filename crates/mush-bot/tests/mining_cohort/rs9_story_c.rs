//! **Doc 41 story (c), the retrieval half, with the bot over SSH** (§4.9; cut RS.9c), at 256
//! (Ahuna Flank) and 512 (Urvara Reach): two players, each typing over their own session.
//!
//! - **A**, the owner, holds two damaged prospectors lying beside the site's ramp and posts a
//!   retrieval of each (`+salvage/retrieve <hull> go`) from Ceres.
//! - **B**, a second player, flies a Dray-P to the site, reads the job board (`+salvage/jobs`),
//!   accepts the first job (`+salvage/accept <job> go`), tows the hull and loads it into B's own
//!   bay — retrieval custody (`+bay` reads "Your right: retrieval"), no hand-back on the site —
//!   and sails home to Ceres, where the docking hands her over and posts the delivery, and B is
//!   paid.
//! - Nobody accepts the second job: after its 28 IC days (the world at rate 3,600), the clerk
//!   hands it to Tycho Mutual and A is told.
//!
//! **What is a fixture** (doc 36a §8's seams): God's: the two hulls placed beside the ramp with
//! their power destroyed (`@unit/damage`) and titled to A (`@asset/title`), on the site's index as
//! casualties (`@tacsim/site leave`), A's money (`@econ/mint`), and the world's rate.
//!
//! **The charter half** — the repossession of a dormant lessee's prospector with the wear
//! allowance and a debt beyond the deposit — is `rs9_story_c_charter`'s, through a test-only
//! dormancy window. Story (a)'s run carries (c)'s refit at Stickney Yards.

use std::time::{Duration, Instant};

use mush_bot::mine::Milestone;
use mush_core::ObjId;

use crate::fr7_round::{PROSPECTOR, WEST, cid, placed, powerless_plan};
use crate::grounds_rig::Ground;
use crate::rs9_story_b::{ceres_workshop, enrol, fly, named, tow_in, typed, until};
use crate::{arena, mine};

/// The two players: A the owner, B the acceptor.
#[derive(Clone, Copy)]
pub(crate) struct Pair {
    pub(crate) a: mine::Miner,
    pub(crate) b: mine::Miner,
}

/// The two hulls and their cells.
type Hulls = [(ObjId, (i64, i64)); 2];

/// **The fixture**: two prospectors beside the ramp, powerless, titled to A. Answers the two
/// hulls, their cells and A.
fn fixture(
    engine: &mut mush_core::Engine,
    ground: &Ground,
    pair: Pair,
) -> (Vec<String>, Hulls, ObjId) {
    let a = named(engine, pair.a.character);
    let (x, y) = tacsim::surface::excursion::site_pad_cm(&engine.world, &cid(ground.entry));
    let side = i64::from(tacsim::surface::ground::SITE_CELL_CM);
    let ramp = (x.div_euclid(side), y.div_euclid(side) - 4);
    let cells = [(ramp.0 + 1, ramp.1), (ramp.0 - 1, ramp.1)];
    let mut said = Vec::new();
    let hulls = [
        (
            placed(engine, ground, PROSPECTOR, "Owner Rig", cells[0], WEST),
            cells[0],
        ),
        (
            placed(engine, ground, PROSPECTOR, "Spare Rig", cells[1], WEST),
            cells[1],
        ),
    ];
    for (hull, _) in hulls {
        said.push(mine::staff_line(
            engine,
            &format!("@unit/damage #{}={}", hull.0, powerless_plan()),
        ));
        said.push(mine::staff_line(
            engine,
            &format!("@asset/title #{}=#{}", hull.0, a.0),
        ));
    }
    (said, hulls, a)
}

/// What the fixture made: the two hulls and their cells, A, B and Ceres's workshop room.
type Made = (Hulls, ObjId, ObjId, ObjId);

/// **Story (c)'s retrieval half on `ground`**, the pair `pair`.
#[allow(clippy::too_many_lines)]
pub(crate) async fn story_c(pair: Pair, ground: &'static Ground) {
    let started = Instant::now();
    let site = ground.entry;
    let stamp = |what: &str| println!("rs9c {site}: +{} s {what}", started.elapsed().as_secs());
    let mut made = None;
    let (rig, said) = mine::boot_with(
        &[pair.a, pair.b],
        |cfg| {
            cfg.tacsim.recovery.inspect_s = 30;
            cfg.tacsim.recovery.attach_s = 30;
            cfg.tacsim.recovery.load_s = 60;
        },
        |engine| {
            let (said, hulls, a) = fixture(engine, ground, pair);
            let b = named(engine, pair.b.character);
            made = Some::<Made>((hulls, a, b, ceres_workshop(engine)));
            said
        },
    )
    .await;
    let (hulls, a_id, b_id, workshop) = made.expect("the fixture ran");
    stamp(&format!("the fixture: {} lines", said.len()));
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
    // Both hulls on the site's index as casualties, one staff write at a time.
    for (hull, (x, y)) in hulls {
        god.say(
            &format!("@tacsim/site {site} leave #{} casualty {x},{y}", hull.0),
            "Site record write requested",
        )
        .await;
        let indexed = Instant::now() + Duration::from_secs(60);
        loop {
            god.idle(Duration::from_secs(2)).await;
            let (_, record) = god
                .answer(&format!("@tacsim/site {site}"), &["Objects:"])
                .await;
            if record.contains(&format!("Unit #{}:", hull.0)) {
                break;
            }
            assert!(
                Instant::now() < indexed,
                "#{} never indexed:\n{record}",
                hull.0
            );
        }
    }
    // A's money: a credit account opened and funded by God (A never enrols).
    crate::rs9_story_e::fund(&mut god, a_id).await;
    let (first, second) = (format!("#{}", hulls[0].0.0), format!("#{}", hulls[1].0.0));

    // 1. A posts both jobs.
    for hull in [&first, &second] {
        let (index, said) = typed(
            &mut a,
            &format!("+salvage/retrieve {hull}"),
            &["Post a retrieval"],
        )
        .await;
        assert_eq!(index, Some(0), "the preview of {hull}:\n{said}");
        let (index, said) = typed(
            &mut a,
            &format!("+salvage/retrieve {hull} go"),
            &["posted for"],
        )
        .await;
        assert_eq!(index, Some(0), "A posts {hull}:\n{said}");
    }
    stamp("A posted both jobs");

    // 2. B flies out (enrolled at the boot's rate, as the desk wants) and reads the board.
    let mut lane = enrol(&mut b, pair.b, started).await;
    mine::lift_rate(&mut god, mine::LIFTED_RATE).await;
    fly(&mut b, &mut lane, started, |lane| {
        lane.reached(Milestone::Launched)
    })
    .await;
    stamp("B's rig is down the ramp");
    let board = until(
        &mut b,
        "+salvage/jobs",
        &[first.as_str(), "No retrieval jobs"],
        5,
        Duration::from_secs(60),
    )
    .await;
    let job = board
        .lines()
        .find(|line| line.contains(&first))
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
        &format!("+salvage/accept #{job}"),
        &["Accept the job"],
    )
    .await;
    assert_eq!(index, Some(0), "B's preview:\n{said}");
    let (index, said) = typed(
        &mut b,
        &format!("+salvage/accept #{job} go"),
        &["You accept job"],
    )
    .await;
    assert_eq!(index, Some(0), "B accepts:\n{said}");
    stamp(&format!("B accepted job #{job}"));

    // 3. B tows her and loads her into B's own bay, as retrieval custody.
    tow_in(&mut b, &mut god, site, hulls[0].0).await;
    until(
        &mut b,
        &format!("+bay {first}"),
        &["Your right: retrieval"],
        5,
        Duration::from_secs(10 * 60),
    )
    .await;
    stamp("she is in B's bay under the retrieval");

    // 4. B sails home with her, the tug left on the site; the docking hands her over; B is paid.
    let lifted = mine::lift_rate(&mut god, 3_600).await;
    stamp(&format!("rate: {}", lifted.lines().next().unwrap_or("")));
    let from = b.seen.len();
    let (_, said) = typed(
        &mut b,
        "+ship/lift ceres leave=1",
        &["Clearance", "lifts", "cannot", "refuse"],
    )
    .await;
    println!("rs9c: B> +ship/lift ceres leave=1\n{}", said.trim_end());
    assert!(
        b.watch(
            from,
            &mush_bot::mine::lines::docks(&lane.settings().hull),
            Duration::from_secs(20 * 60)
        )
        .await,
        "B never docked at Ceres:
{}",
        mush_bot::tail(&b.seen, 4 * 1024)
    );
    stamp("B docked at Ceres");
    // The handover: B ashore and in the workshop (God's hand, the `move_to` seam), where B's
    // `+bay/berth` berths her and posts the delivery.
    let (_, said) = typed(&mut b, "+ship/leave", &["leave", "Leave", "aboard", "lock"]).await;
    println!(
        "rs9c: B> +ship/leave
{}",
        said.trim_end()
    );
    let (_, moved) = god
        .answer(
            &format!("@teleport #{}=#{}", b_id.0, workshop.0),
            &[
                "Teleported",
                "teleport",
                "Done",
                "denied",
                "Huh",
                "can't",
                "Office",
            ],
        )
        .await;
    println!("rs9c: God> @teleport B to C14: {}", moved.trim_end());
    b.idle(Duration::from_secs(2)).await;
    let (index, said) = typed(
        &mut b,
        &format!("+bay/berth {first}"),
        &["berthed in the workshop"],
    )
    .await;
    assert_eq!(
        index,
        Some(0),
        "B berths her:
{said}"
    );
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
    let handed = tacsim::surface::retrieval::TOLD_DELIVERED
        .split("{name}")
        .nth(1)
        .and_then(|rest| rest.split("{port}").next())
        .expect("the delivered line's middle")
        .to_owned();
    assert!(
        a.watch(0, &handed, Duration::from_secs(60)).await,
        "A was never told she was handed over:\n{}",
        mush_bot::tail(&a.seen, 4 * 1024)
    );

    // 5. The second job: nobody accepts it; past its 28 IC days it is Tycho Mutual's.
    let house = tacsim::surface::retrieval::TOLD_HOUSE
        .split("{name}")
        .nth(1)
        .expect("the house line's tail")
        .to_owned();
    assert!(
        a.watch(0, &house, Duration::from_secs(30 * 60)).await,
        "the unaccepted job never went to Tycho:\n{}",
        mush_bot::tail(&a.seen, 4 * 1024)
    );
    let (_, board) = typed(&mut a, "+salvage/jobs", &["#", "No retrieval jobs"]).await;
    println!("rs9c: the board after the window:\n{}", board.trim_end());
    stamp("the unaccepted job is Tycho Mutual's");
    for client in [a, b, god] {
        client.close().await;
    }
    drop(rig);
}

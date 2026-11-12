//! **Doc 44 unit MW.15 — SCREEN and SUPPORT flown as delivered orders** (doc 36 cut GC.0's
//! "Not built"; Q-36-42), over the composed world (`driving`'s keyed rig) on both grounds of
//! `grounds_rig`, with the arena's egress pumped by hand as the field stories pump it.
//!
//! GC.0 left two in-source gates (`field::orders::gc0_a_screen_in_a_field_holds_its_bearing`,
//! `gc0_a_support_in_a_field_binds_to_its_partys_unit_only`) and no story: no tacsim test ran a
//! DT roster to its field. `roster_rig::fly` does, so this is that story, through the production
//! command paths:
//!
//! 1. a captain hires Ilya Mendes at Ceres's Slack Line, leases two prospectors, sails to the
//!    site and flies a mining roster down the ramp: the captain leads from rig 1 and Mendes flies
//!    rig 2 (`roster_rig::fly`; a Dray-P's bay holds two rigs);
//! 2. a second captain's armed hull escorts on the same ground and a third party's armed hull
//!    stands off it; the escort fires on the intruder's bearing (`+field/fire`), the site
//!    promotes, and all three parties are admitted;
//! 3. the leader orders slot 2 to screen (`ORDER DT:… 2 SCREEN 90`). The order travels the IC
//!    transport, is delivered into the roster's book, and Mendes's own frame shows it standing:
//!    the rig turns onto the bearing and answers WILCO. The leader drives four cells off; the
//!    screen stays on its cell and its bearing;
//! 4. the leader orders slot 2 to support slot 1 (`ORDER DT:… 2 SUPPORT 1`). Mendes's rig leaves
//!    the cell it screened from, closes to its station on the leader's rig and answers WILCO;
//! 5. a second SCREEN on another bearing turns the rig onto it: the bearing is the order's;
//! 6. a SUPPORT naming the recipient's own slot is answered NEGATIVE (GC.0's gate, in play), and
//!    the screen goes on standing.
//!
//! What the story stands in for:
//! - **the escort and the intruder are fixture hulls with players seated** (CF.2's `armed_on`
//!   and `seat`, as `escort_lease` stands its intruder): nobody fires after the opening order, so
//!   the story never hangs on the field's dice. The roster is the party under test;
//! - **the roster's rigs are stood on their cells by a staff hand** after the launch
//!   (`stand`), as every field story places its units before the field opens.

use mush_core::ObjId;
use tacsim::arena::perception::MindObjective;

use crate::driving_rig::{
    Rig, command, land, on_pad_as_at, run, site_domain, staff_hand, stocked_tuned, warm,
};
use crate::field_host_rig::{armed_on, egress, frame, seat, second_composed, task, until_composed};
use crate::grounds_rig::{AHUNA_FLANK, Ground, Placed, URVARA_REACH};
use crate::roster_rig::{Billet, Flown, Plan, commit, fence, fly, reply};

/// The shipped mining operator at Ceres (`content/crew/ceres.json`).
const MENDES: &str = "Ilya Mendes";

/// Stand `unit` on `entry`'s ground at `cell`, facing `heading` (a staff hand).
fn stand(rig: &mut Rig, entry: &str, unit: ObjId, (cell, heading): Placed) {
    let domain = site_domain(&rig.tw, entry);
    staff_hand(rig, |ctx| {
        let pose =
            tacsim::surface::excursion::cell_pose([cell.0, cell.1], heading).expect("a pose");
        mush_sim::place(ctx, unit, domain, pose).expect("placed");
    });
}

/// Where `who`'s own frame puts its unit, centimetres, and its heading, millidegrees.
fn pose(rig: &Rig, who: ObjId) -> Option<(i64, i64, u32)> {
    frame(rig, who).map(|frame| (frame.pose.x_cm, frame.pose.y_cm, frame.pose.heading_mdeg))
}

/// The order `who`'s own frame shows standing.
fn standing(rig: &Rig, who: ObjId) -> Option<MindObjective> {
    frame(rig, who).and_then(|frame| frame.order.map(|order| order.objective))
}

/// How far a heading is off a bearing, millidegrees.
fn off(heading_mdeg: u32, bearing_mdeg: u32) -> u32 {
    let d = heading_mdeg.abs_diff(bearing_mdeg) % 360_000;
    d.min(360_000 - d)
}

/// The distance between two frames' units, centimetres.
fn apart(rig: &Rig, a: ObjId, b: ObjId) -> Option<i64> {
    let (ax, ay, _) = pose(rig, a)?;
    let (bx, by, _) = pose(rig, b)?;
    let (dx, dy) = (ax - bx, ay - by);
    Some(i64::try_from((dx * dx + dy * dy).unsigned_abs().isqrt()).unwrap_or(i64::MAX))
}

/// A heading counts as on its bearing within two degrees.
const ON_BEARING_MDEG: u32 = 2_000;

/// Whether `who`'s frame shows a screen standing on `bearing` and the rig turned onto it.
fn screens(rig: &Rig, who: ObjId, bearing_mdeg: u32) -> bool {
    standing(rig, who)
        == Some(MindObjective::Screen {
            facing_mdeg: bearing_mdeg,
        })
        && pose(rig, who)
            .is_some_and(|(_, _, heading)| off(heading, bearing_mdeg) <= ON_BEARING_MDEG)
}

/// **MW.15's story on `at`'s ground** (this module's head).
#[allow(clippy::too_many_lines)] // one story, in order
async fn screen_and_support(at: &Ground) {
    let mut rig = stocked_tuned(|cfg| {
        crate::field_host_rig::tune(cfg);
        // Nobody fires after the opening order: the field holds through the story.
        cfg.field.disengage_s = 3_600;
    });

    // (1) The mining roster: the captain leads from rig 1, Mendes flies rig 2.
    let flown: Flown = fly(
        &mut rig,
        at,
        &Plan {
            who: ("Pilot", 1, "Salt Moth"),
            duty: "mining",
            billets: vec![Billet::Leader, Billet::Hired(MENDES)],
            commander: None,
        },
        |_, _| {},
    );
    let leader = flown.organiser;
    let mendes = flown.operator(2);
    let (lead_rig, hired_rig) = (flown.unit(1), flown.unit(2));
    // The leader's rig on the miner's ground and the NPC's seven cells east of it, facing north.
    // A supporter stands off its charge by the doctrine's station, and the kernel's motor ends a
    // drive within its arrival tolerance of the cell it steers for: two cells cover it.
    let station_cm = i64::from(tacsim::arena::doctrine::SUPPORT_STATION_CELLS) * 1_000 + 2_000;
    stand(&mut rig, at.entry, lead_rig, (at.read, 0));
    stand(
        &mut rig,
        at.entry,
        hired_rig,
        ((at.read.0 + 7, at.read.1), 0),
    );

    // (2) An escort from another captain and an intruder, each an armed hull with its player
    // seated; the escort fires on the intruder's bearing and the site promotes.
    let (captain, _, _) = on_pad_as_at(&mut rig, ("Other", 2, "Tally Moth"), at.site(), 0);
    let (stranger, _, _) = on_pad_as_at(&mut rig, ("Third", 3, "Lone Moth"), at.site(), 0);
    let escort = armed_on(&mut rig, at.entry, "Red Hound", at.escort.0, at.escort.1);
    let intruder = armed_on(&mut rig, at.entry, "Grey Hound", at.walker.0, at.walker.1);
    seat(&mut rig, escort, captain);
    seat(&mut rig, intruder, stranger);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second_composed(&mut rig);
    }
    let bearing = if at.walker.0.0 > at.escort.0.0 {
        "E"
    } else {
        "W"
    };
    let (out, queued) = command(&mut rig, captain, &format!("+field/fire {bearing}"));
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 15, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &crate::driving_rig::cid(at.entry))
        .expect("the live record");
    for unit in [lead_rig, hired_rig, escort, intruder] {
        assert!(
            live.units.iter().any(|row| row.unit == unit),
            "#{} is admitted",
            unit.0
        );
    }
    let party = live.party_of(lead_rig).expect("the roster's party");
    assert_eq!(live.party_of(hired_rig), Some(party));
    let others: std::collections::BTreeSet<_> = [escort, intruder]
        .into_iter()
        .filter_map(|unit| live.party_of(unit))
        .collect();
    assert!(
        others.len() == 2 && !others.contains(&party),
        "three parties stand in the field: the roster, the escort and the intruder"
    );
    // Both roster seats have their own frames, and no order stands on the NPC's yet.
    assert!(
        until_composed(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
            frame(rig, leader).is_some() && frame(rig, mendes).is_some()
        })
        .await,
        "the roster's frames arrive"
    );
    assert_eq!(standing(&rig, mendes), None, "no order stands yet");
    let (sx, sy, facing) = pose(&rig, mendes).expect("the NPC's pose");
    assert!(
        off(facing, 90_000) > ON_BEARING_MDEG,
        "the rig does not face east before it is ordered to: {facing}"
    );

    // (3) SCREEN 90 to slot 2: delivered, standing on Mendes's own frame, obeyed, answered.
    let fenced = fence(&mut rig, leader, &flown);
    let _ = commit(&mut rig, leader, &format!("ORDER {fenced} 2 SCREEN 90"));
    let screened = until_composed(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
        screens(rig, mendes, 90_000)
    })
    .await;
    assert!(
        screened,
        "the screen holds its bearing: order {:?}, pose {:?}, reply {:?}",
        standing(&rig, mendes),
        pose(&rig, mendes),
        reply(&mut rig, leader, &flown, 2)
    );
    let answered = until_composed(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
        reply(rig, leader, &flown, 2).is_some_and(|status| status == "WILCO received")
    })
    .await;
    assert!(
        answered,
        "the screen's WILCO comes back on the transport: {:?}",
        reply(&mut rig, leader, &flown, 2)
    );
    // The leader drives four cells off. A screen is no escort: it stays on its cell and bearing.
    let drive = run(
        &mut rig.tw,
        leader,
        &format!("DRIVE {},{}", at.core.0, at.core.1),
    );
    // The kernel's motor ends a drive within its arrival tolerance of the cell, not on it.
    let arrived = until_composed(&mut rig, &mut rx, &mut lines, 90, |rig, _| {
        frame(rig, leader).is_some_and(|frame| {
            frame.speed_cms == 0
                && frame
                    .motor
                    .as_ref()
                    .is_some_and(|motor| motor.outcome.is_some())
        })
    })
    .await;
    assert!(
        arrived,
        "the leader's drive ends: {drive}; {:?}",
        frame(&rig, leader).map(|frame| (frame.cell, frame.motor))
    );
    let (x, y, heading) = pose(&rig, mendes).expect("the screen's pose");
    assert!(
        (x - sx).abs() < 1_000 && (y - sy).abs() < 1_000 && off(heading, 90_000) <= ON_BEARING_MDEG,
        "the screen holds: from ({sx}, {sy}) to ({x}, {y}), heading {heading}"
    );
    let before = apart(&rig, mendes, leader).expect("the range to the leader");
    assert!(
        before > station_cm + 1_000,
        "the leader is well off the NPC's station: {before} cm"
    );

    // (4) SUPPORT 1 to slot 2: Mendes leaves the screen and closes to its station on the
    // leader's rig.
    let _ = commit(&mut rig, leader, &format!("ORDER {fenced} 2 SUPPORT 1"));
    let stationed = until_composed(&mut rig, &mut rx, &mut lines, 120, |rig, _| {
        matches!(
            standing(rig, mendes),
            Some(MindObjective::Support { of: Some(_) })
        ) && apart(rig, mendes, leader).is_some_and(|cm| cm <= station_cm)
    })
    .await;
    let (tx, ty, _) = pose(&rig, mendes).expect("the supporter's pose");
    assert!(
        stationed && (tx - sx).abs() + (ty - sy).abs() >= 2_000,
        "the supporter keeps to its charge: order {:?}, from ({sx}, {sy}) to ({tx}, {ty}), range \
         {:?} cm, reply {:?}",
        standing(&rig, mendes),
        apart(&rig, mendes, leader),
        reply(&mut rig, leader, &flown, 2)
    );
    let answered = until_composed(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
        reply(rig, leader, &flown, 2).is_some_and(|status| status == "WILCO received")
    })
    .await;
    assert!(
        answered,
        "the supporter's WILCO comes back: {:?}",
        reply(&mut rig, leader, &flown, 2)
    );

    // (5) A second SCREEN on another bearing: the bearing the rig holds is the order's.
    let _ = commit(&mut rig, leader, &format!("ORDER {fenced} 2 SCREEN 270"));
    let turned = until_composed(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
        screens(rig, mendes, 270_000)
    })
    .await;
    assert!(
        turned,
        "the screen turns onto its new bearing: {:?} / {:?}",
        standing(&rig, mendes),
        pose(&rig, mendes)
    );
    let answered = until_composed(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
        reply(rig, leader, &flown, 2).is_some_and(|status| status == "WILCO received")
    })
    .await;
    assert!(answered, "the second screen's WILCO comes back");

    // (6) GC.0's gate in play: support for the recipient's own slot is NEGATIVE, and the order
    // that stood goes on standing.
    let _ = commit(&mut rig, leader, &format!("ORDER {fenced} 2 SUPPORT 2"));
    let refused = until_composed(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
        reply(rig, leader, &flown, 2).is_some_and(|status| status == "NEGATIVE received")
    })
    .await;
    assert!(
        refused,
        "a SUPPORT of its own slot is NEGATIVE: {:?}",
        reply(&mut rig, leader, &flown, 2)
    );
    assert_eq!(
        standing(&rig, mendes),
        Some(MindObjective::Screen {
            facing_mdeg: 270_000
        }),
        "the refused order never reached the kernel"
    );
    assert!(task(&rig).is_some(), "the field still runs");
}

/// **MW.15 at 256** on Ahuna Flank ([`screen_and_support`]).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw15_a_rosters_screen_and_support_are_delivered_and_obeyed() {
    screen_and_support(&AHUNA_FLANK).await;
}

/// **MW.15 at 512** on Urvara Reach ([`screen_and_support`]).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mw15_a_rosters_screen_and_support_are_delivered_and_obeyed_at_512() {
    screen_and_support(&URVARA_REACH).await;
}

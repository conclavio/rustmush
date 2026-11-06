//! **The surrender rig** (doc 42b cut SU.3; §6's stories (s2), (s3), (s5) and the yield (s1)
//! stands on), one script over both grounds of `grounds_rig`. Every item is `pub(crate)`; each
//! domain root declares this module by path beside `custody_rig`.
//!
//! The scene: a stranger's carrier, the Red Moth, on the pad and the stranger's armed hull, the
//! Red Hound, west of the content's raider, launched beside its ramp. The raider's barge leaves
//! the ground before the field (its carrier lost, Q-42b-4's gate). The stranger marks the raider
//! Hostile through the ordinary door, and the field runs with each crew's morale tested by
//! `lift_check`.
#![allow(dead_code)]

use mush_core::{GOD, ObjId};
use tacsim::arena::ledger::LedgerEvent;
use tacsim::field::{CrewLocation, crew_of};
use tacsim::surface::site_record::LeftCondition;

use crate::custody_rig::quick;
use crate::driving_rig::{cid, command, on_pad_as_at, run, staff_hand, stocked_tuned, warm};
use crate::field_host_rig::{armed_on, egress, seat, site_record_at, task, until};
use crate::field_raiders_rig::{raider_hulls, stripped};
use crate::grounds_rig::Ground;

/// **Which of §6's stories runs.**
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    /// The yield (the field half of (s1)): the first hull's crew at resolve 0 yields, the
    /// second's at 1,000 fights on.
    Yields,
    /// (s2) Morale holds: both crews at 1,000, nothing yields.
    Holds,
    /// (s3) No holder, no yield: the Red Moth leaves the ground too, so nobody on the site has a
    /// carrier to take the crew; the crew at resolve 0 fights on and no step is spent.
    NoHolder,
    /// (s5) A restart: the field crashes as soon as the yield is marked; the hull is out of the
    /// field after the restart, and its step and its yield are what they were.
    Restart,
    /// (s4) The window (doc 42b cut SU.5, Q-42b-20): nobody loads the yielded hull. A minute short
    /// of `yield_hold_s` past the demotion the crew is still aboard; at it, the crew leaves play
    /// and the hull stays a casualty.
    Lapse,
}

/// The hull's raider attribute.
fn mark(rig: &crate::driving_rig::Rig, hull: ObjId) -> serde_json::Value {
    rig.tw
        .world()
        .attr(hull, tacsim::field::raiders::RAIDER_ATTR)
        .and_then(|attr| attr.as_json().cloned())
        .unwrap_or_default()
}

/// Whether `site`'s running field holds `unit`.
fn in_field(rig: &crate::driving_rig::Rig, entry: &str, unit: ObjId) -> bool {
    tacsim::field::live_on(rig.tw.world(), &cid(entry))
        .is_some_and(|live| live.units.iter().any(|row| row.unit == unit))
}

/// Whether `entry`'s field has demoted.
fn settled(rig: &mut crate::driving_rig::Rig, entry: &str) -> bool {
    task(rig).is_none() && site_record_at(rig, entry).is_some_and(|r| r.open_field().is_none())
}

/// **The story** on `at`'s ground (this module's head).
#[allow(clippy::too_many_lines)]
pub(crate) async fn surrender(at: &Ground, case: Case) {
    let mut rig = stocked_tuned(quick);
    let (b, b_row, _) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), at.site(), 1);
    let b_carrier = ObjId(b_row.hull);
    let seeded = run(&mut rig.tw, GOD, &format!("@field/raiders {}", at.raider));
    assert!(seeded.contains("seeded"), "{seeded}");
    let launched = run(
        &mut rig.tw,
        GOD,
        &format!("@field/raiders/launch {}", at.raider),
    );
    assert!(launched.contains("launch from"), "{launched}");
    let hulls = raider_hulls(&rig);
    assert_eq!(hulls.len(), 2, "the raider's two hulls stand");
    let barge = tacsim::field::raiders::carrier_of(rig.tw.world(), hulls[0]).expect("a carrier");
    let ramp = tacsim::field::raiders::ramp_cell(rig.tw.world(), barge).expect("a ramp");
    let hound = armed_on(
        &mut rig,
        at.entry,
        "Red Hound",
        (ramp[0] - 7, ramp[1]),
        90_000,
    );
    seat(&mut rig, hound, b);
    let crews: Vec<ObjId> = hulls
        .iter()
        .map(|hull| {
            tacsim::field::crew::seated_in(rig.tw.world(), *hull).expect("each hull has its crew")
        })
        .collect();
    let resolves = match case {
        Case::Holds => [1_000, 1_000],
        Case::Yields | Case::NoHolder | Case::Restart | Case::Lapse => [0, 1_000],
    };
    staff_hand(&mut rig, |ctx| {
        for (crew, resolve) in crews.iter().zip(resolves) {
            assert!(tacsim::field::crew::set_resolve_for_tests(
                ctx,
                *crew,
                Some(resolve)
            ));
        }
    });
    // The first hull fights stripped (224 per mille): its damage term weighs every check.
    stripped(&mut rig, hulls[0]);
    // The carrier lost: the barge is off the ground before the field opens.
    staff_hand(&mut rig, |ctx| {
        mush_sim::unplace(ctx, barge, "test: the raider's barge is gone").expect("unplaced");
    });
    if case == Case::NoHolder {
        staff_hand(&mut rig, |ctx| {
            mush_sim::unplace(ctx, b_carrier, "test: the Red Moth is gone").expect("unplaced");
        });
    }
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let (out, queued) = command(&mut rig, b, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| task(rig)
            .is_some())
        .await,
        "the field opens"
    );
    let entry = at.entry;
    let yielded =
        |rig: &crate::driving_rig::Rig| tacsim::field::raiders::yielded(rig.tw.world(), hulls[0]);

    match case {
        Case::Yields | Case::Restart | Case::Lapse => {
            assert!(
                until(&mut rig, &mut rx, &mut lines, 60, |rig, _| yielded(rig)).await,
                "the first crew yields: {}",
                mark(&rig, hulls[0])
            );
            let before = mark(&rig, hulls[0]);
            assert_eq!(
                before["yielded"]["carrier"].as_u64(),
                Some(u64::from(barge.0)),
                "the yield remembers its carrier: {before}"
            );
            assert!(
                before.get("carrier").is_none(),
                "and is no longer its: {before}"
            );
            assert!(before["morale_step"].as_u64().is_some_and(|step| step >= 1));
            if case == Case::Restart {
                rig = crate::driving_rig::crash(rig, &mut rx, &[b]).await;
                rx = egress(&mut rig);
                assert_eq!(
                    mark(&rig, hulls[0]),
                    before,
                    "the restart keeps the step and the yield"
                );
            }
            assert!(
                until(&mut rig, &mut rx, &mut lines, 40, |rig, _| task(rig)
                    .is_none()
                    || !in_field(rig, entry, hulls[0]))
                .await,
                "the yielded hull leaves the field"
            );
            assert_eq!(mark(&rig, hulls[0])["morale_step"], before["morale_step"]);
            let record = crew_of(rig.tw.world(), crews[0]).expect("the crew's record");
            assert!(
                matches!(
                    record.location,
                    CrewLocation::Unit { unit } | CrewLocation::Stranded { unit, .. }
                        if unit == hulls[0]
                ),
                "the crew checked in where the hull stands: {record:?}"
            );
            assert_eq!(record.resolve_permille, Some(0));
            assert!(
                !tacsim::field::raiders::yielded(rig.tw.world(), hulls[1]),
                "the crew at 1,000 never yields"
            );
            assert!(mark(&rig, hulls[1]).get("morale_step").is_none());
            assert!(
                until(&mut rig, &mut rx, &mut lines, 150, |rig, _| settled(
                    rig, entry
                ))
                .await,
                "the field demotes"
            );
            let left = site_record_at(&mut rig, entry)
                .and_then(|record| record.units.into_iter().find(|row| row.unit == hulls[0]));
            assert_eq!(
                left.map(|row| row.condition),
                Some(LeftCondition::Casualty),
                "the yielded hull stands on the site as a casualty (Q-42b-12)"
            );
            let listed = run(&mut rig.tw, GOD, &format!("@tacsim/site {entry}"));
            assert!(
                listed.contains(&format!("Unit #{}: casualty, yielded at", hulls[0].0)),
                "the site's listing marks it yielded (Q-42b-13): {listed}"
            );
            if case == Case::Lapse {
                let left_ic = site_record_at(&mut rig, entry)
                    .and_then(|record| record.units.into_iter().find(|row| row.unit == hulls[0]))
                    .map(|row| row.left_ic)
                    .expect("the casualty's row");
                let hold = i64::from(tacsim::TacsimConfig::default().field.yield_hold_s);
                assert_eq!(hold, 28 * 86_400, "the window is the 28-IC-day floor");
                let _ = crate::driving_rig::to_ic(&mut rig, left_ic + hold - 60);
                let _ = crate::field_host_rig::second(&mut rig);
                assert!(
                    crew_of(rig.tw.world(), crews[0]).is_some(),
                    "a minute short of the window the crew still waits"
                );
                let _ = crate::driving_rig::to_ic(&mut rig, left_ic + hold);
                assert!(
                    crate::custody_rig::seconds_until(&mut rig, 30, |rig| crew_of(
                        rig.tw.world(),
                        crews[0]
                    )
                    .is_none()),
                    "at the window the crew leaves play: {:?}",
                    crew_of(rig.tw.world(), crews[0])
                );
                assert!(
                    rig.tw
                        .world()
                        .get(crews[0])
                        .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING)),
                    "the crew member is out of the world"
                );
                assert_eq!(
                    site_record_at(&mut rig, entry)
                        .and_then(|record| record
                            .units
                            .into_iter()
                            .find(|row| row.unit == hulls[0]))
                        .map(|row| row.condition),
                    Some(LeftCondition::Casualty),
                    "the hull stays a casualty for the salvage loop"
                );
            }
        }
        Case::Holds | Case::NoHolder => {
            let _ = until(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
                yielded(rig) || settled(rig, entry)
            })
            .await;
            for hull in &hulls {
                assert!(
                    !tacsim::field::raiders::yielded(rig.tw.world(), *hull),
                    "#{} fights on",
                    hull.0
                );
            }
            // (s3): no holder, no draw, so no step is spent either.
            if case == Case::NoHolder {
                assert!(
                    mark(&rig, hulls[0]).get("morale_step").is_none(),
                    "no step drawn without a holder: {}",
                    mark(&rig, hulls[0])
                );
            }
        }
    }
}

/// **(s1) The stranded raid yields, and its crew is taken in** (doc 42b cut SU.4; Q-42b-17 to
/// Q-42b-19) on `at`'s ground. The raider's first hull stands beside the stranger's ramp,
/// powerless, its crew at resolve 0; its barge and its second hull are gone (SU.3's stories keep a
/// crew at 1,000). A staff hand rebuilds it as a drone, since no shipped rig can tow a 30 t line
/// tank (the cut notes' gap). The field opens and the crew yields; the field settles with the hull
/// a casualty. The stranger's tug, launched after the demotion, tows it into the Red Moth: the crew
/// is `Captured`, listed by `+ship/captives`, and refused `+ship/release` on the site (no life
/// support). The Red Moth docks at Ceres: the authority takes the crew into processing with no fee
/// charged, and at release the crew leaves play, the record gone and the object `GOING`.
#[allow(clippy::too_many_lines)]
pub(crate) async fn taken_in(at: &Ground) {
    use crate::custody_rig::{attach_on, load_on, powerless, ramp_cell_on, seconds_until};
    let mut rig = stocked_tuned(quick);
    let (b, b_row, b_tvs) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), at.site(), 1);
    let b_carrier = ObjId(b_row.hull);
    let tug = b_tvs[0];
    let ramp = ramp_cell_on(&rig, at.entry);
    let seeded = run(&mut rig.tw, GOD, &format!("@field/raiders {}", at.raider));
    assert!(seeded.contains("seeded"), "{seeded}");
    let launched = run(
        &mut rig.tw,
        GOD,
        &format!("@field/raiders/launch {}", at.raider),
    );
    assert!(launched.contains("launch from"), "{launched}");
    let hulls = raider_hulls(&rig);
    let barge = tacsim::field::raiders::carrier_of(rig.tw.world(), hulls[0]).expect("a carrier");
    let hull = hulls[0];
    // No shipped rig can tow a 30 t line tank (a 6 t tug takes 12 t, an 8 t escort 16 t): a staff
    // hand rebuilds the yielding hull as a drone, a combat hull the stranger's tug can take (and
    // the departure carries as cargo, doc 36a lane 3). Doc 42b's cut notes record the gap.
    let removed = run(&mut rig.tw, GOD, &format!("@unit/remove #{}", hull.0));
    let created = run(
        &mut rig.tw,
        GOD,
        &format!("@unit/create #{}=tacsim.unit.drone_mk1", hull.0),
    );
    assert!(
        mush_tactics::module::unit_of(rig.tw.world(), hull).is_some(),
        "the hull is rebuilt as a drone: {removed} / {created}"
    );
    crate::driving_rig::to_cell_on(&mut rig, at.entry, hull, (ramp.0 + 1, ramp.1));
    powerless(&mut rig, hull);
    let crews: Vec<ObjId> = hulls
        .iter()
        .map(|hull| {
            tacsim::field::crew::seated_in(rig.tw.world(), *hull).expect("each hull has its crew")
        })
        .collect();
    let crew = crews[0];
    staff_hand(&mut rig, |ctx| {
        for (who, resolve) in crews.iter().zip([0, 1_000]) {
            assert!(tacsim::field::crew::set_resolve_for_tests(
                ctx,
                *who,
                Some(resolve)
            ));
        }
        mush_sim::unplace(ctx, barge, "test: the raider's barge is gone").expect("unplaced");
        // The second hull goes with its barge: the yielding hull stands alone, so nothing of the
        // raider's fires on the Red Moth on its pad (SU.3's stories keep the crew at 1,000).
        mush_sim::unplace(ctx, hulls[1], "test: the second hull is gone").expect("unplaced");
    });
    let hound = armed_on(
        &mut rig,
        at.entry,
        "Red Hound",
        (ramp.0 - 8, ramp.1),
        90_000,
    );
    seat(&mut rig, hound, b);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let (out, queued) = command(&mut rig, b, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the promotion lands");
    let entry = at.entry;
    assert!(
        until(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
            tacsim::field::raiders::yielded(rig.tw.world(), hull)
        })
        .await,
        "the crew yields: {}",
        mark(&rig, hull)
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 180, |rig, _| settled(
            rig, entry
        ))
        .await,
        "the field settles"
    );
    assert_eq!(
        site_record_at(&mut rig, entry)
            .and_then(|record| record.units.into_iter().find(|row| row.unit == hull))
            .map(|row| row.condition),
        Some(LeftCondition::Casualty),
        "the yielded hull lies a casualty"
    );

    // The stranger's tug, launched once the field is over, takes the yielded hull in: the crew is
    // captured by CR.1's branch.
    // A few seconds first, so the close the host still owes lands before the launch's own acts.
    for _ in 0..5 {
        let _ = crate::field_host_rig::second(&mut rig);
    }
    crate::driving_rig::launched_at(&mut rig, at.entry, b, tug);
    crate::driving_rig::to_cell_on(&mut rig, at.entry, tug, (ramp.0 + 1, ramp.1 + 1));
    let _ = crate::field_host_rig::second(&mut rig);
    attach_on(&mut rig, at.entry, (b, 1), tug, hull);
    load_on(&mut rig, at.entry, (b, 1), tug, hull);
    let record = crew_of(rig.tw.world(), crew)
        .cloned()
        .expect("the crew's record");
    assert!(
        matches!(record.location, CrewLocation::Captured { carrier, captor, .. }
            if carrier == b_carrier && captor == b),
        "the crew is held aboard the Red Moth: {record:?}"
    );
    assert_eq!(
        record.resolve_permille,
        Some(0),
        "the capture carries the trait"
    );
    let name = rig
        .tw
        .world()
        .get(crew)
        .map(|o| o.name.clone())
        .expect("a name");
    let held = run(&mut rig.tw, b, "+ship/captives");
    assert!(held.contains(&name), "{held}");
    // Q-42b-19: no life support on open ground.
    let refused = run(&mut rig.tw, b, &format!("+ship/release #{}", crew.0));
    assert!(
        refused.contains("has no way off this ground; hand them over at a port"),
        "{refused}"
    );
    assert!(matches!(
        crew_of(rig.tw.world(), crew).map(|c| &c.location),
        Some(CrewLocation::Captured { .. })
    ));

    // Q-42b-18: the Red Moth docks at Ceres; the authority takes the crew, and no fee is charged.
    let ticket =
        crate::driving_rig::file_and_launch(&mut rig, b, b_carrier, "+ship/lift ceres leave=1");
    let _ = crate::driving_rig::to_ic(&mut rig, ticket.plan.depart_ic + ticket.plan.total_s + 60);
    assert!(
        seconds_until(&mut rig, 120, |rig| matches!(
            crew_of(rig.tw.world(), crew).map(|c| &c.location),
            Some(CrewLocation::Processing { .. })
        )),
        "the crew is handed over at Ceres: {:?}",
        crew_of(rig.tw.world(), crew)
    );
    let Some(CrewLocation::Processing {
        port,
        release_due_ic,
        charge,
    }) = crew_of(rig.tw.world(), crew).map(|c| c.location.clone())
    else {
        unreachable!("matched above");
    };
    assert_eq!(port.as_str(), "ceres");
    assert_eq!(charge, None, "an NPC owes no release fee");
    let _ = crate::driving_rig::to_ic(&mut rig, release_due_ic + 1);
    assert!(
        seconds_until(&mut rig, 30, |rig| crew_of(rig.tw.world(), crew).is_none()),
        "the crew leaves play at release: {:?}",
        crew_of(rig.tw.world(), crew)
    );
    assert!(
        rig.tw
            .world()
            .get(crew)
            .is_none_or(|o| o.flags.contains(mush_core::Flags::GOING)),
        "the crew member is out of the world"
    );
    let held = run(&mut rig.tw, b, "+ship/captives");
    assert!(held.contains("You hold nobody aboard."), "{held}");
}

/// **Doc 42b cut SU.7, the yielded hull drives itself aboard** (the owner's ruling on Q-42b-14,
/// 2026-11-05) on `at`'s ground. The content's raider: its first hull, a line tank of 30 t laden,
/// stands beside the stranger's ramp, stripped but mobile, its crew at resolve 0; its barge and its
/// second hull are gone. The crew yields and the field settles with the hull a casualty. The
/// stranger's 6 t tug, launched after the demotion, takes it under escort — no tow rating binds,
/// and every line reads it driving itself — leads it to the Red Moth's ramp, and it drives up into
/// the bay: in the Red Moth's custody with a salvage claim owed, its crew `Captured` aboard by
/// CR.1's branch, the resolve kept.
#[allow(clippy::too_many_lines)]
pub(crate) async fn escorted_aboard(at: &Ground) {
    use crate::custody_rig::{ramp_cell_on, seconds_until};
    let mut rig = stocked_tuned(quick);
    let (b, b_row, b_tvs) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), at.site(), 1);
    let b_carrier = ObjId(b_row.hull);
    let tug = b_tvs[0];
    let ramp = ramp_cell_on(&rig, at.entry);
    let seeded = run(&mut rig.tw, GOD, &format!("@field/raiders {}", at.raider));
    assert!(seeded.contains("seeded"), "{seeded}");
    let launched = run(
        &mut rig.tw,
        GOD,
        &format!("@field/raiders/launch {}", at.raider),
    );
    assert!(launched.contains("launch from"), "{launched}");
    let hulls = raider_hulls(&rig);
    let barge = tacsim::field::raiders::carrier_of(rig.tw.world(), hulls[0]).expect("a carrier");
    let hull = hulls[0];
    crate::driving_rig::to_cell_on(&mut rig, at.entry, hull, (ramp.0 + 1, ramp.1));
    stripped(&mut rig, hull);
    let crews: Vec<ObjId> = hulls
        .iter()
        .map(|hull| {
            tacsim::field::crew::seated_in(rig.tw.world(), *hull).expect("each hull has its crew")
        })
        .collect();
    let crew = crews[0];
    staff_hand(&mut rig, |ctx| {
        for (who, resolve) in crews.iter().zip([0, 1_000]) {
            assert!(tacsim::field::crew::set_resolve_for_tests(
                ctx,
                *who,
                Some(resolve)
            ));
        }
        mush_sim::unplace(ctx, barge, "test: the raider's barge is gone").expect("unplaced");
        mush_sim::unplace(ctx, hulls[1], "test: the second hull is gone").expect("unplaced");
    });
    let hound = armed_on(
        &mut rig,
        at.entry,
        "Red Hound",
        (ramp.0 - 8, ramp.1),
        90_000,
    );
    seat(&mut rig, hound, b);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    let (out, queued) = command(&mut rig, b, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the promotion lands");
    let entry = at.entry;
    assert!(
        until(&mut rig, &mut rx, &mut lines, 60, |rig, _| {
            tacsim::field::raiders::yielded(rig.tw.world(), hull)
        })
        .await,
        "the crew yields: {}",
        mark(&rig, hull)
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 180, |rig, _| settled(
            rig, entry
        ))
        .await,
        "the field settles"
    );
    assert_eq!(
        site_record_at(&mut rig, entry)
            .and_then(|record| record.units.into_iter().find(|row| row.unit == hull))
            .map(|row| row.condition),
        Some(LeftCondition::Casualty),
        "the yielded hull lies a casualty"
    );
    for _ in 0..5 {
        let _ = crate::field_host_rig::second(&mut rig);
    }

    // The escort: the stranger's 6 t tug beside the 30 t hull, which drives itself.
    crate::driving_rig::launched_at(&mut rig, at.entry, b, tug);
    let (_, pose) = mush_sim::pose_of(rig.tw.world(), hull).expect("the hull stands");
    let cell = tacsim::surface::excursion::site_cell(pose.x_cm, pose.y_cm);
    crate::driving_rig::to_cell_on(&mut rig, at.entry, tug, (cell.0, cell.1 + 1));
    let _ = crate::field_host_rig::second(&mut rig);
    let preview = run(&mut rig.tw, b, &format!("+tv/1 tow #{}", hull.0));
    assert!(
        preview.contains("under escort") && preview.contains("drives itself"),
        "the escort, never a tow: {preview}"
    );
    let (out, queued) = command(&mut rig, b, &format!("+tv/1 tow #{} go", hull.0));
    assert_eq!(queued.len(), 1, "one escort start: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the escort start lands");
    assert!(
        seconds_until(&mut rig, 120, |rig| site_record_at(rig, entry)
            .and_then(|record| record.tows.towing(tug).map(|op| op.casualty()))
            == Some(hull)),
        "the escort is set"
    );

    // At the Red Moth's ramp: the Dray-P's 20 t bay cannot take a 30 t tank, so (the owner's
    // ruling, 2026-11-05) its crew come aboard as captives and the hull stays, titled to the captor.
    crate::driving_rig::to_cell_on(&mut rig, at.entry, tug, (ramp.0 + 1, ramp.1 + 1));
    let _ = crate::field_host_rig::second(&mut rig);
    let preview = run(&mut rig.tw, b, "+tv/1 load");
    assert!(
        preview.contains("will not fit the Red Moth's bay")
            && preview.contains("salvage of the unowned"),
        "{preview}"
    );
    let done = run(&mut rig.tw, b, "+tv/1 load go");
    let _ = crate::field_host_rig::second(&mut rig);
    let record = crew_of(rig.tw.world(), crew)
        .cloned()
        .expect("the crew's record");
    assert!(
        matches!(record.location, CrewLocation::Captured { carrier, captor, .. }
            if carrier == b_carrier && captor == b),
        "the crew is held aboard the Red Moth: {record:?} ({done})"
    );
    assert_eq!(record.resolve_permille, Some(0));
    let held = run(&mut rig.tw, b, "+ship/captives");
    let name = rig
        .tw
        .world()
        .get(crew)
        .map(|o| o.name.clone())
        .expect("a name");
    assert!(held.contains(&name), "{held}");
    assert!(
        !tacsim::surface::tow::in_bay_of(rig.tw.world(), hull, b_carrier)
            && mush_sim::pose_of(rig.tw.world(), hull).is_some(),
        "the hull stays on the site"
    );
    let title = rig
        .tw
        .world()
        .module_state::<mush_economy::EconomyStore>()
        .and_then(|store| store.snapshot().asset(hull).map(|a| a.title_owner.clone()));
    assert_eq!(
        title,
        Some(mush_org::PrincipalId::Object(b)),
        "titled to the captor as salvage of the unowned"
    );
    assert!(
        seconds_until(&mut rig, 60, |rig| site_record_at(rig, entry).is_some_and(
            |record| record.tows.towing(tug).is_none() && record.tows.act_of(tug).is_none()
        )),
        "the escort lets it go"
    );

    // The captor posts its retrieval; nobody accepts, so Tycho Mutual takes it (RS.5, RS.6b).
    let posted = run(&mut rig.tw, b, &format!("+salvage/retrieve #{} go", hull.0));
    let job = tacsim::surface::retrieval::order_of(rig.tw.world(), hull)
        .unwrap_or_else(|| panic!("the job is posted: {posted}"));
    let _ = crate::driving_rig::to_ic(&mut rig, job.accept_by_ic + 60);
    assert!(
        seconds_until(&mut rig, 60, |rig| tacsim::surface::retrieval::order_of(
            rig.tw.world(),
            hull
        )
        .is_some_and(
            |o| o.stage == tacsim::surface::retrieval::RetrievalStage::House
        )),
        "Tycho Mutual takes the retrieval: {:?}",
        tacsim::surface::retrieval::order_of(rig.tw.world(), hull)
    );
}

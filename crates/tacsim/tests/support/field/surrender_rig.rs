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
        Case::Yields | Case::NoHolder | Case::Restart => [0, 1_000],
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
        Case::Yields | Case::Restart => {
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
/// Q-42b-19) on `at`'s ground. The stranger's tug is out on the ground behind the Red Hound; the
/// raider's first hull stands beside the stranger's ramp, stripped and powerless, its crew at
/// resolve 0, and its barge is gone. The field opens and the crew yields; the field demotes with
/// the hull a casualty. The stranger's tug tows it into the Red Moth: the crew is `Captured`,
/// listed by `+ship/captives`, and refused `+ship/release` on the site (no life support). The Red
/// Moth docks at Ceres: the authority takes the crew into processing with no fee charged, and at
/// release the crew leaves play, the record gone and the object `GOING`.
#[allow(clippy::too_many_lines)]
pub(crate) async fn taken_in(at: &Ground) {
    use crate::custody_rig::{attach_on, load_on, powerless, ramp_cell_on, seconds_until};
    let mut rig = stocked_tuned(quick);
    let (b, b_row, b_tvs) = on_pad_as_at(&mut rig, ("Second", 2, "Red Moth"), at.site(), 1);
    let b_carrier = ObjId(b_row.hull);
    let tug = b_tvs[0];
    let ramp = ramp_cell_on(&rig, at.entry);
    crate::driving_rig::launched_at(&mut rig, at.entry, b, tug);
    crate::driving_rig::to_cell_on(&mut rig, at.entry, tug, (ramp.0 - 12, ramp.1));
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

    // The stranger's tug takes the yielded hull in: the crew is captured by CR.1's branch.
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

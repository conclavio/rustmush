//! **Doc 32 cut CF.2 — the field host**, over the composed world (`driving`'s keyed rig on
//! Ahuna Flank), with the arena's egress pumped by hand as `match_lifecycle.rs` pumps it.
//!
//! The cut's gate: two players fly fixture-seeded armed hulls on a site's ground (the armed
//! lease is CF.6's, so a staff hand makes the hulls and seats the players); a third player's
//! prospector works the ground beside them. One pilot orders fire at the other's presence row:
//! the site promotes, everyone on the ground is admitted at their surface poses, and the shot
//! resolves in the field's first pulses. The bystander reads as an Unknown contact and is
//! shootable. The server restarts mid-fight while the W-clock is frozen: the field is held
//! through the countdown and resumes from its snapshot when the clock resumes. The fight ends,
//! the field disengages and demotes, the damage persists into the unit records through the IC
//! check-in, and the loser stays where it fell on the site record.

use crate::field_host_rig::*;

/// **CF.2's gate** (doc 32 §6; Q-CF-9, Q-CF-10, Q-CF-14, Q-CF-16, Q-CF-20, Q-CF-24, Q-CF-33,
/// Q-CF-35, Q-CF-36).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn cf2_two_armed_hulls_fight_the_loser_stays_and_a_restart_resumes_after_the_freeze() {
    let mut rig = stocked_tuned(tune);
    // Three players at Ahuna Flank: A and B fly the armed hulls, C's prospector works the next
    // stretch of ground.
    let (a, _, a_tvs) = on_pad(&mut rig, 1);
    let (b, _, b_tvs) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let (c, _, c_tvs) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 1);
    launched(&mut rig, c, c_tvs[0]);
    to_cell(&mut rig, c_tvs[0], (116, 116));
    let bystander = c_tvs[0];
    let hull_a = armed(&mut rig, "Red Hound", (110, 124), 90_000);
    let hull_b = armed(&mut rig, "Grey Hound", (114, 124), 270_000);
    seat(&mut rig, hull_a, a);
    seat(&mut rig, hull_b, b);
    fragile(&mut rig, hull_b);
    let b_before = mush_tactics::module::unit_of(rig.tw.world(), hull_b)
        .expect("B's hull")
        .clone();
    warm(&mut rig);
    let _: (Vec<ObjId>, Vec<ObjId>) = (a_tvs, b_tvs);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    // Let the surface tick publish its presence book.
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let map = run(&mut rig.tw, a, "+tv/map");
    assert!(
        map.contains("Presence:"),
        "A sees flights on the ground: {map}"
    );

    // (1) A's fire order at the flight to the east (B's hull): the site promotes.
    let (out, queued) = command(&mut rig, a, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    let record = site_record(&mut rig).expect("the site's record");
    let open = record.promoted.clone().expect("the field is open");
    assert_eq!(open.ordinal, 1);
    assert!(
        matches!(open.by, PromotedBy::Fire { pilot, shooter, target } if pilot == a && shooter == hull_a && target == hull_b),
        "{:?}",
        open.by
    );
    // A surface act on the promoted site refuses (CF.1's gate stands).
    let drive = run(&mut rig.tw, c, "+tv/1 drive 120,116 go");
    assert!(drive.contains("Field 1 is being fought"), "{drive}");

    // (2) Admission on the next tick: everyone on the ground under the field's mark.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 10, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    for unit in [hull_a, hull_b, bystander] {
        let mark = mush_sim::deployed::mark_of(rig.tw.world(), unit)
            .unwrap_or_else(|| panic!("#{} is in the field", unit.0));
        assert_eq!(mark.match_id, 0, "a field never takes a match's number");
        assert_eq!(
            mark.field.as_ref().map(|field| field.ordinal),
            Some(1),
            "#{} under field 1's mark",
            unit.0
        );
    }

    // (3) The opening shot resolves under the lock rules: A's cockpit locks the contact on B's
    // cell and a round leaves the gun.
    let gun = weapon_of(&rig, hull_a);
    let rounds = |rig: &Rig| -> Option<u32> {
        frame(rig, a).and_then(|frame| {
            frame
                .weapons
                .iter()
                .find(|weapon| weapon.id.as_str() == gun)
                .map(|weapon| weapon.rounds)
        })
    };
    let near = |cell: Option<(u16, u16)>, (cx, cy): (u16, u16)| {
        cell.is_some_and(|(x, y)| x.abs_diff(cx) <= 2 && y.abs_diff(cy) <= 2)
    };
    let contact_at = |rig: &Rig, at: (u16, u16)| {
        frame(rig, a).and_then(|frame| {
            frame
                .contacts
                .iter()
                .find(|contact| near(contact.cell, at))
                .cloned()
        })
    };
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| rounds(rig)
            .is_some())
        .await,
        "A's cockpit frame arrives"
    );
    let full = rounds(&rig).expect("A's gun");
    let opened = until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
        let locked_b = frame(rig, a).is_some_and(|frame| {
            frame.lock.is_some()
                && frame.lock == contact_at(rig, (114, 124)).map(|contact| contact.id)
        });
        locked_b && rounds(rig).is_some_and(|left| left < full)
    })
    .await;
    assert!(
        opened,
        "the fire order's shot leaves the barrel, locked on B's hull"
    );

    // The bystander: a contact nobody marked, neither friend nor foe, and shootable — the
    // kernel never refuses a shot by relation.
    let bystander_contact = {
        let mut seen = None;
        assert!(
            until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
                seen = contact_at(rig, (116, 116));
                seen.is_some()
            })
            .await,
            "A's sensors hold the bystander"
        );
        seen.expect("the bystander's contact")
    };
    assert!(
        matches!(
            bystander_contact.relation,
            Relation::Unknown | Relation::Neutral
        ),
        "a bystander nobody marked: {:?}",
        bystander_contact.relation
    );
    let before = rounds(&rig).expect("A's gun");
    let said = run(
        &mut rig.tw,
        a,
        &format!("LOCK {}", bystander_contact.designator),
    );
    let shot_bystander = until(&mut rig, &mut rx, &mut lines, 30, |rig, _| {
        let _ = run(&mut rig.tw, a, &format!("FIRE {gun}"));
        let locked = frame(rig, a).is_some_and(|frame| frame.lock == Some(bystander_contact.id));
        locked && rounds(rig).is_some_and(|left| left < before)
    })
    .await;
    assert!(shot_bystander, "A fires at the bystander ({said})");

    // (4) The restart mid-fight, frozen: the field is held through the countdown and resumes
    // from its snapshot when the clock resumes.
    tokio::time::sleep(Duration::from_millis(700)).await;
    pump(&mut rig, &mut rx, &mut lines);
    let tasks = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .expect("the registry");
    staff_hand(&mut rig, |ctx| {
        let _ = ctx.freeze_world_time();
    });
    tasks.shutdown(tacsim::arena_bridge::JOIN_TIMEOUT).await;
    while rx.try_recv().is_ok() {}
    let mut rig = reboot(rig, &[a, b, c]);
    let mut rx = egress(&mut rig);
    assert!(
        task(&rig).is_none(),
        "no field runs while the W-clock is frozen"
    );
    assert!(
        site_record(&mut rig)
            .and_then(|record| record.open_field())
            .is_some(),
        "the site is still promoted"
    );
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    assert!(
        task(&rig).is_none(),
        "still held: no tick runs a frozen world"
    );
    staff_hand(&mut rig, |ctx| {
        let _ = ctx.resume_world_time();
    });
    assert!(
        until(&mut rig, &mut rx, &mut lines, 10, |rig, _| task(rig)
            .is_some())
        .await,
        "the field resumes from its snapshot when the clock resumes"
    );
    for unit in [hull_a, hull_b, bystander] {
        assert!(
            mush_sim::deployed::mark_of(rig.tw.world(), unit)
                .is_some_and(|mark| mark.field.is_some()),
            "#{} is still under the field's mark after the restart",
            unit.0
        );
    }

    // Focus never survives a boot: each pilot takes the cockpit back as the resume line says.
    for (pilot, hull) in [(a, hull_a), (b, hull_b)] {
        let said = run(&mut rig.tw, pilot, &format!("+focus #{}/cockpit", hull.0));
        assert!(!said.contains("Usage"), "{said}");
    }
    // A goes back to B's hull and fires until it is a wreck.
    let wrecked = until(&mut rig, &mut rx, &mut lines, 150, |rig, _| {
        if let Some(contact) = contact_at(rig, (114, 124))
            && frame(rig, a).is_some_and(|frame| frame.lock != Some(contact.id))
        {
            let _ = run(&mut rig.tw, a, &format!("LOCK {}", contact.designator));
        }
        let _ = run(&mut rig.tw, a, &format!("FIRE {gun}"));
        frame(rig, b).is_some_and(|frame| frame.wreck)
    })
    .await;
    assert!(wrecked, "B's hull is wrecked by A's fire");

    // (5) Nobody fires any more: the field disengages and demotes.
    assert!(
        until(&mut rig, &mut rx, &mut lines, 30, |rig, _| {
            task(rig).is_none()
                && site_record(rig).is_some_and(|record| record.open_field().is_none())
        })
        .await,
        "the field demotes after its quiet interval"
    );
    assert!(
        lines.iter().any(|line| matches!(
            line.kind,
            EventKind::MatchEnded {
                reason: tacsim::arena::state::EndReason::Disengaged
            }
        )),
        "the black box ends on the disengagement"
    );
    assert!(
        !lines
            .iter()
            .any(|line| matches!(line.kind, EventKind::SortieClosed { .. })),
        "an IC field closes no hull's epoch (Q-H9)"
    );
    let record = site_record(&mut rig).expect("the record");
    assert_eq!(record.ordinal, 1);
    assert!(record.demoted_ic.is_some());
    let left = record
        .units
        .iter()
        .find(|left| left.unit == hull_b)
        .expect("the loser is on the site record");
    assert_eq!(left.condition, LeftCondition::Wreck);
    let fell = mush_sim::pose_of(rig.tw.world(), hull_b)
        .map(|(_, pose)| (pose.x_cm, pose.y_cm))
        .expect("the wreck stands on the ground");
    assert_eq!((left.pose.x_cm, left.pose.y_cm), fell, "where it fell");
    for unit in [hull_a, hull_b, bystander] {
        assert!(
            !mush_sim::deployed::is_deployed(rig.tw.world(), unit),
            "#{} is checked in",
            unit.0
        );
    }
    // The damage persisted into the unit record through the IC check-in (Q-CF-36): a newer
    // revision, a section's internals at their floor — never the check-out snapshot.
    let b_after = mush_tactics::module::unit_of(rig.tw.world(), hull_b)
        .expect("B's hull")
        .clone();
    assert!(
        b_after.revision() > b_before.revision(),
        "the field's damage is a new revision of B's hull"
    );
    assert!(
        b_after.cells().any(|(addr, cell)| matches!(
            addr,
            mush_tactics::CellAddr::Section { cell: id, .. }
                if id.as_str() == tacsim::definitions::unit::INTERNALS_CELL
        ) && cell.current == cell.min),
        "a section of B's hull came home destroyed"
    );
    // And the surface is the surface's again.
    let drive = run(&mut rig.tw, c, "+tv/1 drive 120,116 go");
    assert!(!drive.contains("Field 1 is being fought"), "{drive}");
}

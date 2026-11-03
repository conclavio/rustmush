//! **Doc 36 cut GC.2 — the kernel batch's two field gates**, over the composed world
//! (`driving`'s keyed rig on Ahuna Flank), the arena's egress pumped by hand as `field_host.rs`
//! pumps it.
//!
//! Both are fields nobody fights: a pilot in an armed hull marks the flight beside it Hostile,
//! the site promotes, and no shot is ever fired.
//!
//! - **The unarmed barge** (Q-36-6, the tempo half): the flight is a prospector, a hull that
//!   never carried a gun. The armed hull holds it Hostile inside its reach, which kept a field
//!   `Engaged` before this cut; now its presence alone lets the quiet interval run and the field
//!   demotes at `[tacsim.field].disengage_s`.
//! - **The idle declared field** (Q-36-5): the flight is a second armed hull. The two hold each
//!   other inside their reach the whole time, so the tempo stays `Engaged`; the field still
//!   demotes once no shot has been fired for `[tacsim.field].idle_demote_s`, whatever its
//!   contacts.
//!
//! The kernel halves of GC.2's other two gates (a wreck's cover, a jammed gun through the
//! check-in) are `tacsim::arena`'s own tests; the CF.7 story (`field_story.rs`) shows both in a
//! field.

use mush_core::ObjId;
use tacsim::TacsimConfig;
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::party::Relation;

use crate::driving_rig::{
    command, land, launched, on_pad, on_pad_as, stocked_tuned, to_cell, warm,
};
use crate::field_host_rig::{armed, egress, frame, seat, second, site_record, task, until};

/// The lab tempo, a snapshot every half second, and the two intervals `quiet` and `idle`.
fn tuned(quiet: u32, idle: u32) -> impl FnOnce(&mut TacsimConfig) {
    move |cfg: &mut TacsimConfig| {
        crate::field_host_rig::tune(cfg);
        cfg.field.disengage_s = quiet;
        cfg.field.idle_demote_s = idle;
    }
}

/// **A's mark on the flight to the east**: one promotion, landed, and the field's task open.
async fn declare(
    rig: &mut crate::driving_rig::Rig,
    rx: &mut tokio::sync::mpsc::Receiver<tacsim::arena_task::ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    a: ObjId,
) {
    let (out, queued) = command(rig, a, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(rig, rx, lines, 15, |rig, _| task(rig).is_some()).await,
        "the field's task opens"
    );
}

/// Whether `pilot`'s frame holds a Hostile contact near `at`.
fn holds_hostile(rig: &crate::driving_rig::Rig, pilot: ObjId, (cx, cy): (u16, u16)) -> bool {
    frame(rig, pilot).is_some_and(|frame| {
        frame.contacts.iter().any(|contact| {
            contact.relation == Relation::Hostile
                && contact
                    .cell
                    .is_some_and(|(x, y)| x.abs_diff(cx) <= 1 && y.abs_diff(cy) <= 1)
        })
    })
}

/// Whether the field has demoted: no task, and the record's promotion closed.
fn demoted(rig: &mut crate::driving_rig::Rig) -> bool {
    task(rig).is_none() && site_record(rig).is_some_and(|record| record.open_field().is_none())
}

fn fired(lines: &[LedgerEvent]) -> usize {
    lines
        .iter()
        .filter(|line| matches!(line.kind, EventKind::Fired { .. }))
        .count()
}

fn ended_disengaged(lines: &[LedgerEvent]) -> bool {
    lines.iter().any(|line| {
        matches!(
            line.kind,
            EventKind::MatchEnded {
                reason: tacsim::arena::state::EndReason::Disengaged
            }
        )
    })
}

/// **GC.2's gate** (Q-36-6, the tempo half): an armed hull holding an unarmed hull Hostile in
/// its reach does not hold the field engaged. The idle bound is an hour, so only the quiet
/// interval can end it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc2_an_unarmed_barge_does_not_hold_a_field_engaged() {
    let mut rig = stocked_tuned(tuned(15, 3_600));
    let (a, _, _) = on_pad(&mut rig, 1);
    let (c, _, c_tvs) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 1);
    launched(&mut rig, c, c_tvs[0]);
    // The prospector stands four cells east of the armed hull, well inside its reach.
    to_cell(&mut rig, c_tvs[0], (114, 124));
    let barge = c_tvs[0];
    let hull = armed(&mut rig, "Red Hound", (110, 124), 90_000);
    seat(&mut rig, hull, a);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    declare(&mut rig, &mut rx, &mut lines, a).await;
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| holds_hostile(
            rig,
            a,
            (114, 124)
        ))
        .await,
        "A holds the unarmed hull Hostile inside its reach"
    );
    let ended = until(&mut rig, &mut rx, &mut lines, 60, |rig, _| demoted(rig)).await;
    assert!(
        ended,
        "the field demotes after its quiet interval with the unarmed hull still Hostile"
    );
    assert_eq!(fired(&lines), 0, "nobody fired");
    assert!(
        ended_disengaged(&lines),
        "the black box ends on the disengagement"
    );
    assert!(
        mush_sim::pose_of(rig.tw.world(), barge).is_some(),
        "the unarmed hull still stands on the ground"
    );
}

/// **GC.2's gate** (Q-36-5): two armed hulls holding each other in reach, one marked Hostile,
/// nobody firing. The quiet interval is an hour, so only the idle bound can end it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn gc2_an_idle_declared_field_demotes() {
    let mut rig = stocked_tuned(tuned(3_600, 10));
    let (a, _, _) = on_pad(&mut rig, 1);
    let (b, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let hull_a = armed(&mut rig, "Red Hound", (110, 124), 90_000);
    let hull_b = armed(&mut rig, "Grey Hound", (114, 124), 270_000);
    seat(&mut rig, hull_a, a);
    seat(&mut rig, hull_b, b);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    declare(&mut rig, &mut rx, &mut lines, a).await;
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| holds_hostile(
            rig,
            a,
            (114, 124)
        ))
        .await,
        "A holds B's armed hull Hostile inside its reach"
    );
    let ended = until(&mut rig, &mut rx, &mut lines, 60, |rig, _| demoted(rig)).await;
    assert!(
        ended,
        "the idle field demotes with a Hostile still in reach"
    );
    assert_eq!(fired(&lines), 0, "nobody fired");
    assert!(
        ended_disengaged(&lines),
        "the black box ends on the disengagement"
    );
    let _ = (hull_b, b);
}

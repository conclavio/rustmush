//! **The 2026-10-31 review's field fixes** (doc 32 §15), over the composed world (`driving`'s
//! keyed rig on Ahuna Flank), the arena's egress pumped by hand as `field_host` pumps it.
//!
//! - **CF-1, the crash windows.** The host's `close` and `exit` writes to the site record run on
//!   the K3 lane after the legacy batch that checked the units in, and the two never share a
//!   turn. Each gate drops that K3 write (the injected fault), restarts, and shows the boot owes
//!   it again: a field that ended is never admitted again and keeps its wreck; a unit out at the
//!   ramp is named on the record and its recover passes.
//! - **CF-3a, the cap.** A wreck an earlier field left is admitted to the next and never counted
//!   against `[tacsim.field].max_units`.
//! - **CF-5a, the landing release.** An extraction landing a pilot out of a running field tells
//!   the arena the seat is released: the hull is the mind's.

use mush_core::{AuthoritativeSubmit, GOD, ObjId, ObjKind};
use tacsim::TacsimConfig;
use tacsim::arena::ledger::{EventKind, LedgerEvent};
use tacsim::arena::state::Controller;
use tacsim::arena_task::ArenaOut;
use tacsim::field::{CrewCondition, CrewLocation, crew_of};
use tacsim::surface::site_record::LeftCondition;
use tokio::sync::mpsc::Receiver;

use crate::driving::{
    AHUNA, Rig, ahuna_domain, cid, command, drain, land, launched, on_pad, on_pad_as, reboot,
    retune, run, staff_hand, stocked_tuned, to_cell, warm,
};
use crate::field_host::{
    armed, egress, fragile, frame, pump, seat, second, site_record, task, until, weapon_of,
};

/// A fast field and a short extraction (`field_host`'s tune and `crew_exit`'s extraction).
fn tune(cfg: &mut TacsimConfig) {
    crate::field_host::tune(cfg);
    cfg.field.extract_s = 60;
}

/// Whether `submit` is the field host's own `op` write to the site record.
fn host_write(submit: &AuthoritativeSubmit, op: &str) -> bool {
    submit.action.key().id.as_str() == tacsim::field::FIELD_ACTION
        && submit.parameters.get("op").and_then(|op| op.as_str()) == Some(op)
}

/// **One world second with the K3 lane down for `op`** (the injected fault): the ticks due, every
/// submission landed but the host's `op` writes, which are dropped; answers how many it dropped.
fn second_dropping(rig: &mut Rig, op: &str) -> usize {
    let t = rig.tw.world().now().saturating_add(1);
    rig.tw.set_now(t);
    let _ = rig.tw.fire_due(t.saturating_mul(1_000));
    let mut dropped = 0;
    for submit in drain(&mut rig.tw) {
        if host_write(&submit, op) {
            dropped += 1;
        } else {
            let _ = land(rig, &submit);
        }
    }
    dropped
}

/// [`until`], every tick through [`second_dropping`]; answers whether `done` held and how many
/// `op` writes were dropped on the way.
async fn until_dropping(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    op: &str,
    secs: u64,
    mut done: impl FnMut(&mut Rig) -> bool,
) -> (bool, usize) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    let (mut beat, mut dropped) = (0_u32, 0);
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        pump(rig, rx, lines);
        beat += 1;
        if beat.is_multiple_of(8) {
            dropped += second_dropping(rig, op);
        }
        if done(rig) {
            return (true, dropped);
        }
    }
    (false, dropped)
}

/// **The restart**: every task stopped and its egress drained, then [`reboot`].
async fn crash(rig: Rig, rx: &mut Receiver<ArenaOut>, reconnect: &[ObjId]) -> Rig {
    let tasks = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .map(|registry| registry.tasks().clone())
        .expect("the registry");
    tasks.shutdown(tacsim::arena_bridge::JOIN_TIMEOUT).await;
    while rx.try_recv().is_ok() {}
    reboot(rig, reconnect)
}

fn in_field(rig: &Rig, unit: ObjId) -> bool {
    mush_sim::deployed::mark_of(rig.tw.world(), unit).is_some_and(|mark| mark.field.is_some())
}

/// A fires at the hull to the east until it is a wreck.
async fn wreck_east(
    rig: &mut Rig,
    rx: &mut Receiver<ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    a: ObjId,
    b: ObjId,
    gun: &str,
) -> bool {
    let near = |cell: Option<(u16, u16)>| {
        cell.is_some_and(|(x, y)| x.abs_diff(114) <= 2 && y.abs_diff(124) <= 2)
    };
    // B flies from a remote console and never takes `+focus`, so B's own frames are not the
    // witness: the black box's wreck line is.
    until(rig, rx, lines, 150, |rig, lines| {
        if let Some(contact) = frame(rig, a).and_then(|frame| {
            frame
                .contacts
                .iter()
                .find(|contact| near(contact.cell))
                .cloned()
        }) && frame(rig, a).is_some_and(|frame| frame.lock != Some(contact.id))
        {
            let _ = run(&mut rig.tw, a, &format!("LOCK {}", contact.designator));
        }
        let _ = run(&mut rig.tw, a, &format!("FIRE {gun}"));
        lines
            .iter()
            .any(|line| matches!(line.kind, EventKind::UnitDestroyed { unit, .. } if unit == b))
    })
    .await
}

/// **CF-1's close window and CF-3a's cap** (doc 32 §15; Q-CF-20, Q-CF-21, Q-CF-24, Q-CF-26,
/// Q-CF-45). A wrecks B's hull and the field demotes, but its `close` never reaches the site
/// record before the server goes down. The boot finds the closing tombstone: the field is never
/// admitted again, the close lands, and B's wreck is on the record where it fell. Then, with the
/// cap one below everyone on the ground, a second field still admits them all: the wreck is not
/// counted.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn cf_review_a_crash_before_the_close_lands_keeps_the_wreck_and_the_cap_skips_it() {
    let mut rig = stocked_tuned(tune);
    let (a, _, _) = on_pad(&mut rig, 1);
    let (b, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let (c, _, c_tvs) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 1);
    launched(&mut rig, c, c_tvs[0]);
    to_cell(&mut rig, c_tvs[0], (116, 116));
    let hull_a = armed(&mut rig, "Red Hound", (110, 124), 90_000);
    let hull_b = armed(&mut rig, "Grey Hound", (114, 124), 270_000);
    seat(&mut rig, hull_a, a);
    seat(&mut rig, hull_b, b);
    fragile(&mut rig, hull_b);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }

    // Field 1: A's fire order at B's hull.
    let (out, queued) = command(&mut rig, a, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 10, |rig, _| task(rig)
            .is_some())
        .await,
        "field 1's task opens"
    );
    let admitted = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA))
        .expect("field 1's live record")
        .units
        .len();
    let gun = weapon_of(&rig, hull_a);
    let wrecked = wreck_east(&mut rig, &mut rx, &mut lines, a, hull_b, &gun).await;
    assert!(
        wrecked,
        "B's hull is wrecked by A's fire: {} shots, {} hits on B",
        lines
            .iter()
            .filter(|line| matches!(line.kind, EventKind::Fired { unit, .. } if unit == hull_a))
            .count(),
        lines
            .iter()
            .filter(|line| matches!(line.kind, EventKind::Hit { target, .. } if target == hull_b))
            .count()
    );

    // The field disengages and demotes; its close never lands (the K3 lane is down).
    let (ended, dropped) = until_dropping(&mut rig, &mut rx, &mut lines, "close", 40, |rig| {
        task(rig).is_none()
    })
    .await;
    assert!(ended, "field 1 demotes after its quiet interval");
    let (_, more) = until_dropping(&mut rig, &mut rx, &mut lines, "close", 1, |_| false).await;
    assert!(dropped + more > 0, "the host owed the record its close");
    assert!(
        site_record(&mut rig).is_some_and(|record| record.open_field().is_some()),
        "the close never reached the site record"
    );
    assert!(
        !in_field(&rig, hull_b),
        "the check-in landed on the legacy lane"
    );

    // The restart: the boot owes the close again and never admits the finished field.
    let mut rig = crash(rig, &mut rx, &[a, b, c]).await;
    let mut rx = egress(&mut rig);
    let mut reopened = false;
    let closed = until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
        reopened |= task(rig).is_some();
        site_record(rig).is_some_and(|record| record.open_field().is_none())
    })
    .await;
    assert!(!reopened, "a field that ended is never admitted again");
    assert!(closed, "the owed close lands after the restart");
    let record = site_record(&mut rig).expect("the record");
    let left = record
        .units
        .iter()
        .find(|left| left.unit == hull_b)
        .expect("the wreck is on the site record");
    assert_eq!(left.condition, LeftCondition::Wreck);
    let fell = mush_sim::pose_of(rig.tw.world(), hull_b)
        .map(|(_, pose)| (pose.x_cm, pose.y_cm))
        .expect("the wreck stands on the ground");
    assert_eq!((left.pose.x_cm, left.pose.y_cm), fell, "where it fell");

    // CF-3a: everyone field 1 admitted is still on the ground, the wreck among them. With the
    // cap one below that, field 2 admits them all.
    let cap = u32::try_from(admitted - 1).expect("a small field");
    retune(&mut rig, |cfg| cfg.field.max_units = cap);
    let mut rig = crash(rig, &mut rx, &[a, b, c]).await;
    let mut rx = egress(&mut rig);
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let (out, queued) = command(&mut rig, a, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 10, |rig, _| task(rig)
            .is_some())
        .await,
        "field 2 opens under a cap of {cap}: the wreck is not counted"
    );
    let live = tacsim::field::live_on(rig.tw.world(), &cid(AHUNA)).expect("field 2's live record");
    assert_eq!(live.field.ordinal, 2);
    assert_eq!(
        live.units.len(),
        admitted,
        "everyone on the ground is admitted"
    );
    assert!(
        live.units.iter().any(|row| row.unit == hull_b),
        "the wreck is admitted (Q-CF-26)"
    );
}

/// **CF-1's exit window** (doc 32 §15; Q-CF-18). C recovers the prospector out of a running field
/// at its carrier's ramp: it is checked in and out of the live record, but its `exit` never
/// reaches the site record before the server goes down. The field resumes; the boot owes the exit
/// again from the live record, the record names it, and C's recover passes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cf_review_a_crash_before_the_exit_lands_owes_it_again_and_the_recover_passes() {
    let mut rig = stocked_tuned(tune);
    let (a, _, _) = on_pad(&mut rig, 1);
    let (b, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let (c, _, c_tvs) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 1);
    let prospector = c_tvs[0];
    launched(&mut rig, c, prospector);
    let hull_a = armed(&mut rig, "Red Hound", (110, 124), 90_000);
    let hull_b = armed(&mut rig, "Grey Hound", (114, 124), 270_000);
    seat(&mut rig, hull_a, a);
    seat(&mut rig, hull_b, b);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let (out, queued) = command(&mut rig, a, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 10, |rig, _| task(rig)
            .is_some())
        .await,
        "the field's task opens"
    );
    assert!(in_field(&rig, prospector), "the prospector is in the field");

    // The exit: checked in at the ramp, its K3 write dropped.
    let asked = run(&mut rig.tw, c, "+tv/1 recover go");
    assert!(asked.contains("makes for the ramp"), "{asked}");
    let (out, dropped) = until_dropping(&mut rig, &mut rx, &mut lines, "exit", 20, |rig| {
        !in_field(rig, prospector)
    })
    .await;
    assert!(out, "the prospector is checked in at the ramp");
    let (_, more) = until_dropping(&mut rig, &mut rx, &mut lines, "exit", 2, |_| false).await;
    assert!(dropped + more > 0, "the host owed the record its exit");
    assert!(
        !site_record(&mut rig).is_some_and(|record| record.exited(prospector)),
        "the exit never reached the site record"
    );

    // The restart: the field resumes, and the exit is owed again from the live record.
    let mut rig = crash(rig, &mut rx, &[a, b, c]).await;
    let mut rx = egress(&mut rig);
    let named = until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
        site_record(rig).is_some_and(|record| record.exited(prospector))
    })
    .await;
    assert!(named, "the owed exit lands after the restart");
    assert!(task(&rig).is_some(), "the field resumed and carries on");
    assert!(
        tacsim::field::live_on(rig.tw.world(), &cid(AHUNA))
            .is_some_and(|live| live.owed_exits.is_empty()),
        "the live record forgets the exit once it landed"
    );
    // C's own recover passes the field's gate: the record names the exit.
    let again = run(&mut rig.tw, c, "+tv/1 recover go");
    assert!(!again.contains("being fought"), "{again}");
}

/// **A hull of the design `id`** on Ahuna Flank's ground at `cell` facing `heading`.
fn hull_of(rig: &mut Rig, id: &str, name: &str, cell: (i64, i64), heading: u32) -> ObjId {
    let domain = ahuna_domain(&rig.tw);
    staff_hand(rig, |ctx| {
        let spec = ctx
            .w
            .module_state::<tacsim::TacsimStore>()
            .expect("the layer")
            .definitions()
            .bindings
            .get(&cid(id))
            .map(|binding| binding.spec.clone())
            .unwrap_or_else(|| panic!("{id} is bound"));
        let obj = ctx
            .w
            .create(ObjKind::Thing, name, GOD, None)
            .expect("the hull's object");
        mush_tactics::module::create_unit(ctx, obj, &spec).expect("the hull's unit record");
        let pose =
            tacsim::surface::excursion::cell_pose([cell.0, cell.1], heading).expect("a site pose");
        mush_sim::place(ctx, obj, domain, pose).expect("placed on the ground");
        obj
    })
}

/// **Every control section of `unit` destroyed** (a fixture: its pilot is incapacitated in the
/// field's first frames, Q-CF-38).
fn control_destroyed(rig: &mut Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let cells = mush_tactics::module::unit_of(ctx.w, unit)
            .expect("the unit")
            .clone();
        let ops: Vec<mush_tactics::UnitMutationOp> = cells
            .cells()
            .filter_map(|(addr, cell)| match addr {
                mush_tactics::CellAddr::Section { section, cell: id }
                    if tacsim::field::crew::CONTROL_SECTIONS.contains(&section.as_str())
                        && id.as_str() == tacsim::definitions::unit::INTERNALS_CELL =>
                {
                    Some(mush_tactics::UnitMutationOp::SetCell {
                        addr: addr.clone(),
                        value: cell.min,
                    })
                }
                _ => None,
            })
            .collect();
        assert!(!ops.is_empty(), "the design has a control section");
        let plan = mush_tactics::UnitMutationPlan::new(
            ops,
            None,
            Some(smol_str::SmolStr::new_static("test:cf-review-control")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// The controller the world's mirror of `task` holds for `unit`.
fn controller(rig: &Rig, task: tacsim::arena_task::MatchId, unit: ObjId) -> Option<Controller> {
    rig.tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.mirror(task))
        .and_then(|mirror| mirror.controllers.get(&unit).copied())
}

/// **CF-5a, the landing release** (doc 32 §15; Q-CF-38, Q-CF-39). B flies a biped from inside it,
/// its head destroyed: incapacitated in the field's first frames, B asks for extraction. When the
/// extraction lands, the field is still running and the arena is told B's seat is released: the
/// world's mirror hands the hull to its mind and B is no pilot of the task.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cf_review_an_extraction_out_of_a_running_field_releases_the_seat() {
    let mut rig = stocked_tuned(tune);
    let (a, _, _) = on_pad(&mut rig, 1);
    let (b, _, _) = on_pad_as(&mut rig, "Other", 2, "Tally Moth", 1);
    let hull_a = armed(&mut rig, "Red Hound", (110, 124), 90_000);
    let hull_b = hull_of(
        &mut rig,
        "tacsim.unit.biped_mk1",
        "Grey Strider",
        (114, 124),
        270_000,
    );
    seat(&mut rig, hull_a, a);
    seat(&mut rig, hull_b, b);
    staff_hand(&mut rig, |ctx| {
        ctx.w.move_to(b, hull_b).expect("B climbs into the hull");
    });
    control_destroyed(&mut rig, hull_b);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines = Vec::new();
    for _ in 0..3 {
        let _ = second(&mut rig);
    }
    let (out, queued) = command(&mut rig, a, "+field/fire E");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    land(&mut rig, &queued[0]).expect("the promotion lands");
    let incapacitated = until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
        crew_of(rig.tw.world(), b).is_some_and(|crew| {
            crew.condition == CrewCondition::Incapacitated
                && crew.location == CrewLocation::Unit { unit: hull_b }
        })
    })
    .await;
    assert!(
        incapacitated,
        "B is incapacitated in the field's first frames"
    );
    let field = task(&rig).expect("the field runs");
    assert_eq!(
        controller(&rig, field, hull_b),
        Some(Controller::Pilot),
        "B's seat is the pilot's"
    );

    let asked = run(&mut rig.tw, b, "+sortie/extract");
    assert!(asked.contains("Extraction requested"), "{asked}");
    let due = crew_of(rig.tw.world(), b)
        .and_then(|crew| crew.extraction.as_ref().map(|coming| coming.due_ic))
        .expect("the extraction's due instant");
    while rig.tw.world().world_ic_now().at < due {
        let _ = second(&mut rig);
    }
    let extracted = until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
        crew_of(rig.tw.world(), b)
            .is_some_and(|crew| matches!(crew.location, CrewLocation::Port { .. }))
    })
    .await;
    assert!(extracted, "B is at port after the extraction delay");
    assert_eq!(task(&rig), Some(field), "the field carries on");
    assert_eq!(
        controller(&rig, field, hull_b),
        Some(Controller::Mind),
        "the arena was told the seat is released: the hull is the mind's"
    );
    let seated = rig
        .tw
        .world()
        .module_state::<tacsim::arena_bridge::ArenaRegistry>()
        .and_then(|registry| registry.seat_of_pilot(b));
    assert_eq!(seated, None, "B is no pilot of the field's task");
}

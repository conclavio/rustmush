//! **Doc 36 cut GC.4 — the tow (story a)** (Q-36-12, Q-36-13, Q-36-14, Q-36-15, Q-36-20,
//! Q-36-21, Q-36-54), over the composed world (`driving`'s keyed rig on Ahuna Flank), the
//! arena's egress pumped by hand as `field_host.rs` pumps it.
//!
//! The main path of story (a): a casualty of the captain's own two-rig party lies off the ramp
//! after a field; the wingman takes it in tow on the IC tick, a restart keeps the attachment, a
//! second field on the site admits the tow coupled (the tug at the tow rate, the casualty its
//! load), the wingman drags it to the ramp during that field, the field settles with the casualty
//! at the tug's cell, and the wingman loads it into the carrier's bay — a restart while loading
//! keeps the load — before the carrier lifts.
//!
//! **What is a fixture.** The casualty's power is destroyed by staff hand before the first
//! field, standing in for the round that disabled it (GC.3's precedent): the first field then
//! indexes it as the casualty it is, and everything after runs the production paths. The
//! casualty's launch and the tow's landings go through the composed path (doc 36a lane 4:
//! `launched_composed`, `seconds_until_with`), so its black box holds the epoch's open line and
//! the load's `RecoveredToBase` close line is asserted.

use mush_core::ObjId;
use mush_roll::commerce::ic_claims::IcCloseKind;
use tacsim::arena::ledger::LedgerEvent;
use tacsim::surface::black_box::BlackBoxLine;
use tacsim::surface::site_record::LeftCondition;

use crate::driving_rig::{
    AHUNA, act_go, cid, command, finish, launched_composed, on_pad, on_pad_as, ramp_at, reboot,
    record_of, run, staff_hand, stocked_tuned, to_cell, warm,
};
use crate::field_gc3::black_box;
use crate::field_host_rig::{
    armed, egress, seat, second, second_composed, site_record, task, tune, until,
};

/// **Destroy `unit`'s power component** (the staff fixture standing in for the round that
/// disabled it): no power is no control, a casualty.
pub(crate) fn powerless(rig: &mut crate::driving_rig::Rig, unit: ObjId) {
    staff_hand(rig, |ctx| {
        let plan = mush_tactics::UnitMutationPlan::new(
            vec![mush_tactics::UnitMutationOp::SetComponentCondition {
                component: mush_tactics::unit::ComponentId::new("power").expect("an id"),
                condition: mush_tactics::ConditionId::new("destroyed").expect("a condition"),
            }],
            None,
            Some(smol_str::SmolStr::new_static("test:gc4-powerless")),
        )
        .expect("a plan");
        mush_tactics::module::apply_plan(ctx, unit, &plan).expect("applied");
    });
}

/// Whether the field has demoted: no task, and the record's promotion closed.
pub(crate) fn demoted(rig: &mut crate::driving_rig::Rig) -> bool {
    task(rig).is_none() && site_record(rig).is_some_and(|record| record.open_field().is_none())
}

/// `unit`'s site cell, when it stands on the ground.
pub(crate) fn cell_of(rig: &crate::driving_rig::Rig, unit: ObjId) -> Option<(i64, i64)> {
    mush_sim::pose_of(rig.tw.world(), unit)
        .map(|(_, at)| (at.x_cm.div_euclid(1_000), at.y_cm.div_euclid(1_000)))
}

/// Seconds until `done` holds, at most `limit`.
pub(crate) fn seconds_until(
    rig: &mut crate::driving_rig::Rig,
    limit: u32,
    done: impl FnMut(&mut crate::driving_rig::Rig) -> bool,
) -> bool {
    seconds_until_with(rig, limit, false, done)
}

/// [`seconds_until`], the ticks' submissions landed on the composed path when `composed`
/// (`field_host::second_composed`): the tow's finishes then write their black box lines.
pub(crate) fn seconds_until_with(
    rig: &mut crate::driving_rig::Rig,
    limit: u32,
    composed: bool,
    mut done: impl FnMut(&mut crate::driving_rig::Rig) -> bool,
) -> bool {
    for _ in 0..limit {
        if done(rig) {
            return true;
        }
        // Only the tow's own landings are this gate's; the field host retries its owed close
        // until it forgets it, and a retry after the landing is refused by design.
        let landed = if composed {
            second_composed(rig)
        } else {
            second(rig)
        };
        let errors: Vec<String> = landed
            .into_iter()
            .filter(|error| error.contains(tacsim::surface::tow::TOW_ACTION))
            .collect();
        assert!(errors.is_empty(), "a tow landing refused: {errors:?}");
    }
    done(rig)
}

/// The tug's tow, as the site's record holds it.
pub(crate) fn towing(rig: &mut crate::driving_rig::Rig, tug: ObjId) -> Option<ObjId> {
    site_record(rig).and_then(|record| record.tows.towing(tug).map(|op| op.casualty()))
}

/// **GC.4's gate: story (a)'s main path.**
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn gc4_the_wingman_drags_the_casualty_to_the_ramp_and_loads_it_before_the_lift() {
    let mut rig = stocked_tuned(|cfg| {
        tune(cfg);
        cfg.field.disengage_s = 90;
        cfg.field.idle_demote_s = 3_600;
        cfg.recovery.inspect_s = 30;
        cfg.recovery.attach_s = 30;
        cfg.recovery.load_s = 60;
    });
    // The captain's party: a Dray-P on Ahuna Flank's pad and two rigs, the escort (rig 1) and its
    // wingman (rig 2).
    let (a, row, tvs) = on_pad(&mut rig, 2);
    let carrier = ObjId(row.hull);
    let (escort, wingman) = (tvs[0], tvs[1]);
    launched_composed(&mut rig, a, escort);
    let epoch = record_of(&mut rig.tw, escort)
        .and_then(|record| record.sortie)
        .map(|open| {
            assert!(!open.unrecorded, "the composed launch writes the open line");
            open.id
        })
        .expect("the escort's epoch is open on its rig record");
    let _ = act_go(&mut rig, a, "+tv/2 launch go");
    let _ = finish(&mut rig, wingman, a);
    let (rx, ry, _) = ramp_at(&rig.tw, AHUNA);
    let ramp = (rx.div_euclid(1_000), ry.div_euclid(1_000));
    to_cell(&mut rig, escort, (ramp.0 + 4, ramp.1));
    to_cell(&mut rig, wingman, (ramp.0 + 5, ramp.1));
    powerless(&mut rig, escort);
    // A stranger's armed hull west of the pad: its pilot's mark opens each field.
    let (b, _, _) = on_pad_as(&mut rig, "Second", 2, "Red Moth", 1);
    let hull = armed(&mut rig, "Red Hound", (ramp.0 - 8, ramp.1), 90_000);
    seat(&mut rig, hull, b);
    seat(&mut rig, wingman, a);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();

    // Field 1: the stranger marks east; nobody fires; it settles with the escort a casualty.
    let (out, queued) = command(&mut rig, b, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| task(rig)
            .is_some())
        .await,
        "the first field opens"
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 180, |rig, _| demoted(rig)).await,
        "the first field settles"
    );
    let record = site_record(&mut rig).expect("the site record");
    assert!(
        record
            .units
            .iter()
            .any(|left| left.unit == escort && left.condition == LeftCondition::Casualty),
        "the escort lies a casualty: {:?}",
        record.units
    );

    // The tow on the IC tick (Q-36-12, Q-36-13): the preview is the inspection; go files one act.
    let preview = run(&mut rig.tw, a, "+tv/2 tow");
    assert!(preview.contains("In tow it moves at"), "{preview}");
    let (out, queued) = command(&mut rig, a, "+tv/2 tow go");
    assert_eq!(queued.len(), 1, "one tow start: {out}");
    assert_eq!(
        queued[0].action.key().id.as_str(),
        tacsim::surface::tow::TOW_ACTION
    );
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the tow start lands");
    let act = site_record(&mut rig)
        .and_then(|record| record.tows.act_of(wingman).cloned())
        .expect("the tow act is on the site's record");
    assert!(
        act.due_ic > act.started_ic,
        "inspecting and attaching take time"
    );
    // A tug with a tow act in flight does nothing else.
    let busy = run(&mut rig.tw, a, "+tv/2 scan");
    assert!(busy.contains("busy with its tow"), "{busy}");
    assert!(
        seconds_until_with(&mut rig, 120, true, |rig| towing(rig, wingman)
            == Some(escort)),
        "the tow attaches at its due instant"
    );
    assert!(
        site_record(&mut rig).is_some_and(|record| record.tows.act_of(wingman).is_none()),
        "the act is spent"
    );

    // A restart while attached keeps the attachment (story a).
    let mut rig = reboot(rig, &[a, b]);
    let _ = second(&mut rig);
    // The pilots take their seats again after the restart (the fixture's remote cabs).
    seat(&mut rig, hull, b);
    seat(&mut rig, wingman, a);
    assert_eq!(
        towing(&mut rig, wingman),
        Some(escort),
        "the attachment survives the restart"
    );
    let speed = site_record(&mut rig)
        .and_then(|record| record.tows.towing(wingman).map(|op| op.coupled_speed_cms()))
        .expect("the tow's rate");
    assert!(
        speed > 0 && speed < 300,
        "the tow is slower than the rig alone: {speed}"
    );
    // A tug in tow only drives, releases or loads.
    let held = run(&mut rig.tw, a, "+tv/2 scan");
    assert!(held.contains("has a casualty in tow"), "{held}");

    // Field 2: promotion admits the tow coupled (Q-36-20), and the wingman drags the casualty to
    // the ramp during the field.
    let mut rx = egress(&mut rig);
    let (out, queued) = command(&mut rig, b, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| task(rig)
            .is_some())
        .await,
        "the second field opens"
    );
    assert!(
        tacsim::field::field_of_unit(rig.tw.world(), wingman).is_some(),
        "the tug is in the field"
    );
    assert!(
        tacsim::field::field_of_unit(rig.tw.world(), escort).is_none(),
        "the casualty rides as its tug's load"
    );
    assert!(
        until(&mut rig, &mut rx, &mut lines, 20, |rig, _| {
            tacsim::surface::tow::admitted_speed_cms(rig.tw.world(), &cid(AHUNA), wingman).is_some()
        })
        .await,
        "the field's snapshot shows the tug"
    );
    assert_eq!(
        tacsim::surface::tow::admitted_speed_cms(rig.tw.world(), &cid(AHUNA), wingman),
        Some(speed),
        "the tug drives at the tow rate on wall time"
    );
    let drive = run(&mut rig.tw, a, &format!("DRIVE {},{}", ramp.0, ramp.1));
    assert!(!drive.contains("cannot"), "{drive}");
    assert!(
        until(&mut rig, &mut rx, &mut lines, 240, |rig, _| demoted(rig)).await,
        "the second field settles"
    );
    // The casualty stands at its tug's cell, still in tow, indexed there.
    let tug_cell = cell_of(&rig, wingman).expect("the tug is on the ground");
    assert_eq!(
        cell_of(&rig, escort),
        Some(tug_cell),
        "the casualty was set down at its tug"
    );
    assert!(
        (tug_cell.0 - ramp.0).abs() <= 1 && (tug_cell.1 - ramp.1).abs() <= 1,
        "the wingman stands beside the ramp: {tug_cell:?}, ramp {ramp:?}; DRIVE said: {drive}"
    );
    assert_eq!(towing(&mut rig, wingman), Some(escort), "still in tow");
    let record = site_record(&mut rig).expect("the site record");
    let indexed = record
        .units
        .iter()
        .find(|left| left.unit == escort)
        .expect("the casualty is still indexed");
    assert_eq!(indexed.pose.cell(), tug_cell, "indexed at its tug's cell");

    // The load (Q-36-15, Q-36-54): beside the ramp, into the carrier's bay; a restart while
    // loading keeps the load.
    let _ = second(&mut rig);
    let preview = run(&mut rig.tw, a, "+tv/2 load");
    assert!(preview.contains("into the Salt Moth's bay"), "{preview}");
    let (out, queued) = command(&mut rig, a, "+tv/2 load go");
    assert_eq!(queued.len(), 1, "one load start: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the load start lands");
    assert!(
        site_record(&mut rig).is_some_and(|record| record.tows.act_of(wingman).is_some()),
        "the load is in flight"
    );
    let mut rig = reboot(rig, &[a, b]);
    assert!(
        site_record(&mut rig).is_some_and(|record| record.tows.act_of(wingman).is_some()),
        "the load survives the restart"
    );
    assert!(
        seconds_until_with(&mut rig, 120, true, |rig| {
            tacsim::surface::tow::in_bay_of(rig.tw.world(), escort, carrier)
                && site_record(rig).is_some_and(|record| record.tows.act_of(wingman).is_none())
        }),
        "the casualty is loaded"
    );
    // The hull came home as it was; the site no longer lists it; its epoch closed.
    let record = site_record(&mut rig).expect("the site record");
    assert!(
        record.units.iter().all(|left| left.unit != escort),
        "off the index"
    );
    assert!(record.tows.towing(wingman).is_none(), "the tow is spent");
    let cells = mush_tactics::module::unit_of(rig.tw.world(), escort).expect("the unit");
    let state = tacsim::field_ops::assess_tacsim_unit(cells, None)
        .expect("assessed")
        .state;
    assert_ne!(
        state,
        tacsim::field_ops::OperationalState::Operational,
        "recovery never heals"
    );
    let escort_record = record_of(&mut rig.tw, escort).expect("the escort's rig record");
    assert!(escort_record.sortie.is_none(), "its epoch is closed");
    assert!(escort_record.excursion.is_none(), "its excursion is closed");
    // The load's close line, on the black box: `RecoveredToBase` into the carrier, by the tug.
    let closed = black_box(&rig, escort)
        .into_iter()
        .rev()
        .find_map(|line| match line {
            BlackBoxLine::SortieClosed {
                sortie,
                close,
                recovered_by,
                ..
            } => Some((sortie, close, recovered_by)),
            BlackBoxLine::MindSealed { .. } => None,
        });
    assert_eq!(
        closed,
        Some((
            epoch,
            IcCloseKind::RecoveredToBase { base: carrier },
            Some(wingman)
        )),
        "the black box closes the epoch recovered to the carrier by the wingman"
    );

    // The wingman comes home under its own power, and the carrier lifts with both.
    let _ = second(&mut rig);
    let _ = act_go(&mut rig, a, "+tv/2 recover go");
    let _ = finish(&mut rig, wingman, a);
    assert!(tacsim::surface::tow::in_bay_of(
        rig.tw.world(),
        wingman,
        carrier
    ));
    let (out, queued) = command(&mut rig, a, "+ship/lift ceres");
    assert!(!queued.is_empty(), "the carrier lifts: {out}");
}

/// **The bay's measures** (Q-36-15, Q-36-54): the Dray-P's bay takes 20 t and 70 m3 of hulls
/// beside its two-rig cap, the prospector stows at its authored 24 m3, and an empty bay has room
/// for exactly what its limits say.
#[test]
fn gc4_the_bay_counts_mass_and_volume_as_well_as_rigs() {
    let mut rig = stocked_tuned(tune);
    let (_, row, tvs) = on_pad(&mut rig, 2);
    let carrier = ObjId(row.hull);
    let full = tacsim::surface::tow::bay_room_of(rig.tw.world(), carrier, None).expect("a bay");
    assert_eq!(full.items_free, 0, "two rigs fill a two-rig bay");
    assert_eq!(full.mass_free_kg, 20_000 - 12_000);
    assert_eq!(full.volume_free_l, 70_000 - 48_000);
    let one_out =
        tacsim::surface::tow::bay_room_of(rig.tw.world(), carrier, Some(tvs[0])).expect("a bay");
    assert_eq!(one_out.items_free, 1);
    assert!(
        one_out.check_can_load(6_000, 24_000).is_ok(),
        "a prospector fits"
    );
    assert!(
        one_out.check_can_load(15_000, 24_000).is_err(),
        "a hull past the bay's mass does not"
    );
    assert!(
        one_out.check_can_load(6_000, 50_000).is_err(),
        "a hull past the bay's volume does not"
    );
}

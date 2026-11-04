//! **Doc 36 cut GC.4 — the tow (story a)** (Q-36-12, Q-36-13, Q-36-14, Q-36-15, Q-36-20,
//! Q-36-21, Q-36-54), over the composed world (`driving`'s keyed rig on Ahuna Flank), the
//! arena's egress pumped by hand as `field_host.rs` pumps it.
//!
//! The main path of story (a): a casualty of the captain's own two-rig party lies off the ramp
//! after a field; the wingman takes it in tow on the IC tick, a restart keeps the attachment, a
//! second field on the site admits the tow coupled (the tug at the tow rate; since doc 40 cut
//! FR.5 the casualty a kernel unit trailing it), the wingman drags it to the ramp during that
//! field, the field settles with the casualty at its own trailing cell beside the tug, and the
//! wingman loads it into the carrier's bay — a restart while loading keeps the load — before the
//! carrier lifts. FR.5's own stories (release, a fallen tug, the coupled withdraw at the ramp)
//! are `shard_10/field_fr5.rs`.
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
    AHUNA, act_go, cid, command, finish, launched, launched_composed, on_pad, on_pad_as, ramp_at,
    reboot, record_of, run, staff_hand, stocked_tuned, to_cell, warm,
};
use crate::field_gc3::black_box;
use crate::field_host_rig::{
    armed, egress, seat, second, second_composed, set_section, site_record, task, tune, until,
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

// ---- doc 36a lane 3: P0-8 ------------------------------------------------------------------

/// **Doc 36a lane 3, P0-8**: the tug falls irrecoverable with the casualty in tow. GC.4's ground
/// — the escort a casualty, its wingman attached by `+tv/2 tow go` — then the wingman goes down
/// with everything (GC.3's fixture stands in for the rounds) before the second field. At that
/// field's demotion the tug is remains and off the ground, the tow has ended with it, and the
/// casualty keeps its row on the index at the cell it stands on; before the fix the close read the
/// casualty's pose off its gone tug and dropped the row.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn lane3_a_casualty_keeps_its_row_when_its_tug_falls_irrecoverable() {
    let mut rig = stocked_tuned(lane3_tune);
    let (a, _, tvs) = on_pad(&mut rig, 2);
    let (escort, wingman) = (tvs[0], tvs[1]);
    launched(&mut rig, a, escort);
    let _ = act_go(&mut rig, a, "+tv/2 launch go");
    let _ = finish(&mut rig, wingman, a);
    let (rx, ry, _) = ramp_at(&rig.tw, AHUNA);
    let ramp = (rx.div_euclid(1_000), ry.div_euclid(1_000));
    to_cell(&mut rig, escort, (ramp.0 + 4, ramp.1));
    to_cell(&mut rig, wingman, (ramp.0 + 5, ramp.1));
    powerless(&mut rig, escort);
    let (b, _, _) = on_pad_as(&mut rig, "Second", 2, "Red Moth", 1);
    let hull = armed(&mut rig, "Red Hound", (ramp.0 - 8, ramp.1), 90_000);
    seat(&mut rig, hull, b);
    seat(&mut rig, wingman, a);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();

    // Field 1 settles with the escort a casualty; the wingman takes it in tow.
    field_settles(&mut rig, &mut rx, &mut lines, b).await;
    let (out, queued) = command(&mut rig, a, "+tv/2 tow go");
    assert_eq!(queued.len(), 1, "one tow start: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the tow start lands");
    assert!(
        seconds_until(&mut rig, 120, |rig| towing(rig, wingman) == Some(escort)),
        "the tow attaches"
    );
    let escort_cell = cell_of(&rig, escort).expect("the casualty is on the ground");

    // The tug goes down with everything, still coupled; field 2 opens over it and settles.
    set_section(&mut rig, wingman, "boom", Some("irrecoverable"), None);
    field_settles(&mut rig, &mut rx, &mut lines, b).await;
    assert!(
        mush_sim::pose_of(rig.tw.world(), wingman).is_none(),
        "the tug is remains, off the ground"
    );
    assert_eq!(
        cell_of(&rig, escort),
        Some(escort_cell),
        "the casualty stands where it was held"
    );
    let record = site_record(&mut rig).expect("the site record");
    let row = record
        .units
        .iter()
        .find(|left| left.unit == escort)
        .unwrap_or_else(|| panic!("the casualty keeps its row: {:?}", record.units));
    assert_eq!(row.condition, LeftCondition::Casualty);
    assert_eq!(row.pose.cell(), escort_cell, "indexed where it stands");
    assert!(
        record.units.iter().all(|left| left.unit != wingman),
        "an irrecoverable tug is no casualty or wreck on the index: {:?}",
        record.units
    );
    assert!(
        record.tows.towing(wingman).is_none(),
        "the tow ended with the tug"
    );
    assert!(
        record.objects.iter().any(|object| matches!(
            object.kind,
            tacsim::surface::site_record::GroundKind::Remains { of: Some(of), .. } if of == wingman
        )),
        "the tug's remains lie where it fell: {:?}",
        record.objects
    );
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
    // Doc 40 cut FR.5 (Q-40-7): the casualty is a kernel unit of its tug's party, trailing it,
    // where GC.4 carried it off the kernel's ground as the tug's load.
    assert_eq!(
        tacsim::field::field_of_unit(rig.tw.world(), escort),
        tacsim::field::field_of_unit(rig.tw.world(), wingman),
        "the casualty is in the field with its tug"
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
    // Doc 40 cut FR.5 (Q-40-7): the casualty stands at its own trailing cell beside its tug,
    // never on it, still in tow, indexed there; GC.4 set it down on the tug's cell.
    let tug_cell = cell_of(&rig, wingman).expect("the tug is on the ground");
    let escort_cell = cell_of(&rig, escort).expect("the casualty is on the ground");
    assert_ne!(escort_cell, tug_cell, "the casualty is not on its tug");
    assert!(
        (escort_cell.0 - tug_cell.0).abs() <= 1 && (escort_cell.1 - tug_cell.1).abs() <= 1,
        "the casualty trails beside its tug: {escort_cell:?}, tug {tug_cell:?}"
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
    // Doc 40 cut FR.5: indexed at its own cell, where GC.4 indexed it at its tug's.
    assert_eq!(indexed.pose.cell(), escort_cell, "indexed where it stands");

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

// ---- doc 36a §11 #9 ---------------------------------------------------------------------

/// **Doc 36a §11 #9**: a casualty one field left stands through a second field over the
/// same ground as an uncounted entrant; the second demotion indexes it again, and its row keeps
/// the instant the first left it — where before the fix every demotion restarted the 28-day
/// write-off clock (doc 36 cut GC.5), so a rig abandoned on day 0 and fought over on day 27
/// waited until day 55.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lane3_a_later_field_never_restarts_the_write_off_clock() {
    let mut rig = stocked_tuned(lane3_tune);
    let (a, _, tvs) = on_pad(&mut rig, 2);
    let (escort, wingman) = (tvs[0], tvs[1]);
    launched(&mut rig, a, escort);
    let _ = act_go(&mut rig, a, "+tv/2 launch go");
    let _ = finish(&mut rig, wingman, a);
    let (rx, ry, _) = ramp_at(&rig.tw, AHUNA);
    let ramp = (rx.div_euclid(1_000), ry.div_euclid(1_000));
    to_cell(&mut rig, escort, (ramp.0 + 4, ramp.1));
    to_cell(&mut rig, wingman, (ramp.0 + 6, ramp.1));
    powerless(&mut rig, escort);
    let (b, _, _) = on_pad_as(&mut rig, "Second", 2, "Red Moth", 1);
    let hull = armed(&mut rig, "Red Hound", (ramp.0 - 8, ramp.1), 90_000);
    seat(&mut rig, hull, b);
    seat(&mut rig, wingman, a);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();

    // Field 1 leaves the escort a casualty on the index.
    field_settles(&mut rig, &mut rx, &mut lines, b).await;
    let first = site_record(&mut rig).expect("the site record");
    let first_left_ic = first
        .units
        .iter()
        .find(|left| left.unit == escort)
        .unwrap_or_else(|| panic!("the casualty is indexed: {:?}", first.units))
        .left_ic;

    // Field 2 over the same ground: the casualty stands in it, uncounted, and is indexed again.
    field_settles(&mut rig, &mut rx, &mut lines, b).await;
    let second = site_record(&mut rig).expect("the site record");
    assert!(
        second.ordinal > first.ordinal,
        "a second field closed over it: {} after {}",
        second.ordinal,
        first.ordinal
    );
    let row = second
        .units
        .iter()
        .find(|left| left.unit == escort)
        .unwrap_or_else(|| panic!("the casualty keeps its row: {:?}", second.units));
    assert_eq!(
        row.left_ic, first_left_ic,
        "the instant it was first left is kept"
    );
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

// ---- doc 36a lane 3: P0-5 ------------------------------------------------------------------

/// **A combat hull with no logistics** (doc 36a P0-5): the Mk1 ground drone — the one shipped
/// combat unit light enough for a prospector to tow (3 t against the 2× cap; a line tank's
/// 29.9 t is beyond any rig) — a Thing with its unit record, on Ahuna Flank's ground at `cell`,
/// its power destroyed by staff hand so the next field indexes it a casualty.
fn combat_casualty(rig: &mut crate::driving_rig::Rig, name: &str, cell: (i64, i64)) -> ObjId {
    let domain = crate::driving_rig::site_domain(&rig.tw, AHUNA);
    let unit = staff_hand(rig, |ctx| {
        let spec = {
            let definitions = ctx
                .w
                .module_state::<tacsim::TacsimStore>()
                .expect("the layer")
                .definitions();
            let id = cid("tacsim.unit.drone_mk1");
            assert!(
                definitions
                    .specs
                    .get(&id)
                    .is_some_and(|spec| spec.logistics.is_none()),
                "a combat hull carries no logistics"
            );
            definitions
                .bindings
                .get(&id)
                .map(|binding| binding.spec.clone())
                .expect("the drone is bound")
        };
        let obj = ctx
            .w
            .create(mush_core::ObjKind::Thing, name, mush_core::GOD, None)
            .expect("the hull's object");
        mush_tactics::module::create_unit(ctx, obj, &spec).expect("the hull's unit record");
        let pose =
            tacsim::surface::excursion::cell_pose([cell.0, cell.1], 90_000).expect("a site pose");
        mush_sim::place(ctx, obj, domain, pose).expect("placed on the ground");
        obj
    });
    powerless(rig, unit);
    unit
}

/// GC.4's stage timings for the lane's regressions, on the rig's default disengagement (doc 43
/// TS.16): their bodies only wait for each field to settle, so only the gate keeps 90 s.
fn lane3_tune(cfg: &mut tacsim::TacsimConfig) {
    tune(cfg);
    cfg.field.idle_demote_s = 3_600;
    cfg.recovery.inspect_s = 30;
    cfg.recovery.attach_s = 30;
    cfg.recovery.load_s = 60;
}

/// A field opened by `marker`'s hostile mark east, run until it settles.
async fn field_settles(
    rig: &mut crate::driving_rig::Rig,
    rx: &mut tokio::sync::mpsc::Receiver<tacsim::arena_task::ArenaOut>,
    lines: &mut Vec<LedgerEvent>,
    marker: ObjId,
) {
    let (out, queued) = command(rig, marker, "+field/mark E=hostile");
    assert_eq!(queued.len(), 1, "one promotion: {out}");
    crate::driving_rig::land(rig, &queued[0]).expect("the promotion lands");
    assert!(
        until(rig, rx, lines, 20, |rig, _| task(rig).is_some()).await,
        "the field opens"
    );
    assert!(
        until(rig, rx, lines, 180, |rig, _| demoted(rig)).await,
        "the field settles"
    );
}

/// **Doc 36a lane 3, P0-5**: a combat casualty with no logistics, towed and loaded into the
/// carrier's bay through `+tv/1 tow go` and `+tv/1 load go`, weighs its dry mass in the
/// passage's basis — `+ship/plan` reads a plan and `+ship/lift ceres` files — where before the
/// fix every departure of that carrier answered `PLAN_DARK` for good.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lane3_a_loaded_combat_casualty_never_darkens_the_lift() {
    let mut rig = stocked_tuned(lane3_tune);
    let (a, row, tvs) = on_pad(&mut rig, 1);
    let carrier = ObjId(row.hull);
    let tug = tvs[0];
    launched(&mut rig, a, tug);
    let (rx, ry, _) = ramp_at(&rig.tw, AHUNA);
    let ramp = (rx.div_euclid(1_000), ry.div_euclid(1_000));
    to_cell(&mut rig, tug, (ramp.0 + 1, ramp.1));
    let drone = combat_casualty(&mut rig, "Red Drone", (ramp.0 + 2, ramp.1));
    // A stranger's armed hull west of the pad: its pilot's mark opens the field.
    let (b, _, _) = on_pad_as(&mut rig, "Second", 2, "Red Moth", 1);
    let hull = armed(&mut rig, "Red Hound", (ramp.0 - 8, ramp.1), 90_000);
    seat(&mut rig, hull, b);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    field_settles(&mut rig, &mut rx, &mut lines, b).await;
    let record = site_record(&mut rig).expect("the site record");
    assert!(
        record
            .units
            .iter()
            .any(|left| left.unit == drone && left.condition == LeftCondition::Casualty),
        "the drone lies a casualty: {:?}",
        record.units
    );

    // The tow, then the load into the tug's own carrier (the drone is nobody's: custody).
    let _ = second(&mut rig);
    let (out, queued) = command(&mut rig, a, "+tv/1 tow go");
    assert_eq!(queued.len(), 1, "one tow start: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the tow start lands");
    assert!(
        seconds_until(&mut rig, 120, |rig| towing(rig, tug) == Some(drone)),
        "the tow attaches"
    );
    let preview = run(&mut rig.tw, a, "+tv/1 load");
    assert!(preview.contains("into the Salt Moth's bay"), "{preview}");
    let (out, queued) = command(&mut rig, a, "+tv/1 load go");
    assert_eq!(queued.len(), 1, "one load start: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the load start lands");
    assert!(
        seconds_until(&mut rig, 120, |rig| {
            tacsim::surface::tow::in_bay_of(rig.tw.world(), drone, carrier)
                && site_record(rig).is_some_and(|record| record.tows.act_of(tug).is_none())
        }),
        "the drone is loaded"
    );

    // The tug comes home; the carrier's passage still plans, and the lift files.
    let _ = second(&mut rig);
    let _ = act_go(&mut rig, a, "+tv/1 recover go");
    let _ = finish(&mut rig, tug, a);
    assert!(tacsim::surface::tow::in_bay_of(
        rig.tw.world(),
        tug,
        carrier
    ));
    let plan = run(&mut rig.tw, a, "+ship/plan ceres");
    assert!(
        !plan.contains("cannot be planned") && plan.contains("Plan for"),
        "the plan reads with a combat hull in the bay: {plan}"
    );
    let (out, queued) = command(&mut rig, a, "+ship/lift ceres");
    assert!(
        !queued.is_empty(),
        "the carrier lifts with the casualty aboard: {out}"
    );
}

/// **Doc 36a lane 3, P0-5's rider**: a tug whose own carrier is not in reach never loads into
/// the nearest carrier when that carrier is a stranger's. The stranger's Dray-P sits on Urvara
/// Reach's pad; its rig stands beside Ahuna Flank's ramp (a staff placement standing in for the
/// drive over), tows the captain's casualty, and `+tv/1 load` is refused naming the Salt Moth.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lane3_a_load_into_a_strangers_carrier_is_refused() {
    let mut rig = stocked_tuned(lane3_tune);
    let (a, _, a_tvs) = on_pad(&mut rig, 1);
    let escort = a_tvs[0];
    launched(&mut rig, a, escort);
    let (rx, ry, _) = ramp_at(&rig.tw, AHUNA);
    let ramp = (rx.div_euclid(1_000), ry.div_euclid(1_000));
    to_cell(&mut rig, escort, (ramp.0 + 2, ramp.1));
    powerless(&mut rig, escort);
    // The stranger: a carrier on Urvara Reach's pad, its rig stood beside Ahuna Flank's ramp.
    let (b, _, b_tvs) = crate::driving_rig::on_pad_as_at(
        &mut rig,
        ("Second", 2, "Red Moth"),
        ("urvara", "urvara_port"),
        1,
    );
    let tug = b_tvs[0];
    crate::driving_rig::launched_at(&mut rig, "urvara_port", b, tug);
    crate::driving_rig::to_cell_on(&mut rig, AHUNA, tug, (ramp.0 + 1, ramp.1));
    // A third party's armed hull west of the pad: its pilot's mark opens the field.
    let (c, _, _) = on_pad_as(&mut rig, "Third", 3, "Lone Moth", 1);
    let hull = armed(&mut rig, "Red Hound", (ramp.0 - 8, ramp.1), 90_000);
    seat(&mut rig, hull, c);
    warm(&mut rig);
    let mut rx = egress(&mut rig);
    let mut lines: Vec<LedgerEvent> = Vec::new();
    field_settles(&mut rig, &mut rx, &mut lines, c).await;
    assert!(
        site_record(&mut rig).is_some_and(|record| record
            .units
            .iter()
            .any(|left| left.unit == escort && left.condition == LeftCondition::Casualty)),
        "the escort lies a casualty"
    );

    // The stranger's rig takes it in tow; the only carriers in reach are strangers' to it.
    let _ = second(&mut rig);
    let (out, queued) = command(&mut rig, b, "+tv/1 tow go");
    assert_eq!(queued.len(), 1, "one tow start: {out}");
    crate::driving_rig::land(&mut rig, &queued[0]).expect("the tow start lands");
    assert!(
        seconds_until(&mut rig, 120, |rig| towing(rig, tug) == Some(escort)),
        "the tow attaches"
    );
    let refused = run(&mut rig.tw, b, "+tv/1 load");
    assert!(
        refused.contains("may only load into its own carrier's bay or a Friend's"),
        "{refused}"
    );
    let (out, queued) = command(&mut rig, b, "+tv/1 load go");
    assert!(queued.is_empty(), "nothing is filed: {out}");
    assert!(
        out.contains("may only load into its own carrier's bay or a Friend's"),
        "{out}"
    );
}
